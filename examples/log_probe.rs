//! Executable G1 smoke probe, not a second supervisor or production launch path.
//! Descriptor 0 receives stdout and descriptor 1 receives stderr in collector mode.
#[cfg(unix)]
mod probe {
    use apphost::logs::{LogWriter, READ_CHUNK, Stream};
    use std::{
        fs::{self, File},
        io::{self, BufRead, Read, Write},
        os::{
            fd::{AsRawFd, FromRawFd, OwnedFd},
            unix::{net::UnixStream, process::CommandExt},
        },
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    fn completion(root: &Path) -> PathBuf {
        root.with_extension("done")
    }
    fn release(root: &Path) -> PathBuf {
        root.with_extension("release")
    }
    fn result(root: &Path) -> PathBuf {
        root.with_extension("producer")
    }

    fn collect(root: &Path) -> io::Result<()> {
        let mut writer = LogWriter::create(root.to_owned(), 64 * 1024, 4)?;
        // Owned descriptors were installed by Command, never accepted from user JSON.
        let mut inputs = unsafe { [File::from_raw_fd(0), File::from_raw_fd(1)] };
        let mut polls = inputs.each_ref().map(|file| libc::pollfd {
            fd: file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        });
        let mut buffer = [0; READ_CHUNK];
        while polls.iter().any(|p| p.fd >= 0) {
            let rc = unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as _, -1) };
            if rc < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            for index in 0..inputs.len() {
                if polls[index].fd < 0 || polls[index].revents == 0 {
                    continue;
                }
                match inputs[index].read(&mut buffer) {
                    Ok(0) => polls[index].fd = -1,
                    Ok(count) => writer.append(
                        "fixture",
                        if index == 0 {
                            Stream::Stdout
                        } else {
                            Stream::Stderr
                        },
                        &buffer[..count],
                    )?,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(error),
                }
            }
        }
        drop(writer);
        fs::write(completion(root), b"complete")
    }

    fn supervise(root: &Path) -> io::Result<()> {
        let executable = std::env::current_exe()?;
        let (out_reader, out_writer) = UnixStream::pair()?;
        let (err_reader, err_writer) = UnixStream::pair()?;
        let mut collector = Command::new(&executable)
            .arg("--collect")
            .arg(root)
            .stdin(Stdio::from(OwnedFd::from(out_reader)))
            .stdout(Stdio::from(OwnedFd::from(err_reader)))
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        let producer = Command::new(&executable)
            .arg("--produce")
            .arg(root)
            .stdin(Stdio::null())
            .stdout(Stdio::from(OwnedFd::from(out_writer)))
            .stderr(Stdio::from(OwnedFd::from(err_writer)))
            .process_group(0)
            .spawn();
        let mut producer = match producer {
            Ok(child) => child,
            Err(error) => {
                let _ = collector.kill();
                let _ = collector.wait();
                return Err(error);
            }
        };
        println!("{} {}", collector.id(), producer.id());
        io::stdout().flush()?;
        let watched_root = root.to_owned();
        let monitor = thread::spawn(move || -> io::Result<()> {
            collector.wait()?;
            fs::write(watched_root.with_extension("collector-exited"), b"exited")
        });
        producer.wait()?;
        monitor
            .join()
            .map_err(|_| io::Error::other("Collector monitor panicked"))??;
        Ok(())
    }

    fn produce(root: &Path) -> io::Result<()> {
        io::stdout().write_all(b"before\n")?;
        io::stderr().write_all(b"stderr-before\n")?;
        wait_for(|| release(root).exists())?;
        let write = io::stdout().write_all(b"after\x00\xff\n");
        let outcome = match write {
            Ok(()) => "ok".into(),
            Err(error) => format!("{:?}", error.kind()),
        };
        fs::write(result(root), outcome)?;
        Ok(())
    }

    #[track_caller]
    fn wait_for(mut predicate: impl FnMut() -> bool) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(8);
        while !predicate() {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "Probe deadline exceeded at {}",
                        std::panic::Location::caller()
                    ),
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    struct Fixture {
        supervisor: Child,
        pids: Vec<u32>,
        root: PathBuf,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.supervisor.kill();
            let _ = self.supervisor.wait();
            // Orphans cannot be waited as Child handles. Match the unique test path
            // before signalling, so a recycled PID cannot target an unrelated task.
            for pid in &self.pids {
                if let Ok(output) = Command::new("/bin/ps")
                    .args(["-p", &pid.to_string(), "-o", "command="])
                    .output()
                {
                    let command = String::from_utf8_lossy(&output.stdout);
                    if command.contains(self.root.to_string_lossy().as_ref())
                        && command.contains("log_probe")
                    {
                        unsafe {
                            libc::kill(*pid as i32, libc::SIGKILL);
                        }
                    }
                }
            }
        }
    }

    fn fixture(root: &Path) -> io::Result<Fixture> {
        let mut supervisor = Command::new(std::env::current_exe()?)
            .arg("--supervise")
            .arg(root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let mut line = String::new();
        io::BufReader::new(supervisor.stdout.take().unwrap()).read_line(&mut line)?;
        let pids: Vec<u32> = line
            .split_whitespace()
            .map(|s| s.parse().unwrap())
            .collect();
        if pids.len() != 2 {
            let _ = supervisor.kill();
            let _ = supervisor.wait();
            return Err(io::Error::other("Supervisor failed to start fixture"));
        }
        let fixture = Fixture {
            supervisor,
            pids,
            root: root.to_owned(),
        };
        wait_for(|| {
            fs::read(root.join("0/fixture.out")).is_ok_and(|bytes| bytes.starts_with(b"before"))
        })?;
        Ok(fixture)
    }

    fn smoke() -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("survives-supervisor");
        let mut fixture = fixture(&root)?;
        fixture.supervisor.kill()?;
        fixture.supervisor.wait()?;
        fs::write(release(&root), b"go")?;
        wait_for(|| completion(&root).exists())?;
        assert_eq!(
            fs::read(root.join("0/fixture.out"))?,
            b"before\nafter\x00\xff\n"
        );
        assert_eq!(fs::read(root.join("0/fixture.err"))?, b"stderr-before\n");
        assert_eq!(fs::read_to_string(result(&root))?, "ok");
        drop(fixture);
        println!(
            "supervisor SIGKILL: producer output and independent collector survive; binary bytes preserved"
        );

        let root = temp.path().join("collector-crash");
        let mut fixture = self::fixture(&root)?;
        let rc = unsafe { libc::kill(fixture.pids[0] as i32, libc::SIGKILL) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        // The parent reaps its own child; no ps/zombie timing assumption.
        wait_for(|| root.with_extension("collector-exited").exists())?;
        fs::write(release(&root), b"go")?;
        wait_for(|| result(&root).exists())?;
        assert_eq!(fs::read_to_string(result(&root))?, "BrokenPipe");
        wait_for(|| fixture.supervisor.try_wait().is_ok_and(|s| s.is_some()))?;
        println!(
            "collector SIGKILL: producer sees BrokenPipe; failure policy requires an explicit product decision"
        );
        Ok(())
    }

    pub fn run() -> io::Result<()> {
        let args: Vec<_> = std::env::args_os().collect();
        if args.len() == 1 {
            return smoke();
        }
        if args.len() != 3 {
            return Err(io::Error::other("Expected MODE ROOT"));
        }
        let root = Path::new(&args[2]);
        match args[1].to_str() {
            Some("--collect") => collect(root),
            Some("--supervise") => supervise(root),
            Some("--produce") => produce(root),
            _ => Err(io::Error::other("Unknown probe mode")),
        }
    }
}

fn main() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        probe::run()
    }
    #[cfg(not(unix))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "This process probe requires Unix",
        ))
    }
}
