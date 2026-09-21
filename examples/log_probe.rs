//! Executable G1 smoke probe, not a second supervisor or production launch path.
//! Descriptor 0 receives stdout and descriptor 1 receives stderr in collector mode.
#[cfg(target_os = "macos")]
mod probe {
    use apphost::logs::{LogWriter, READ_CHUNK, Stream};
    use apphost::process::{Group, observe_exit};
    use apphost::runtime::{InstanceStatus, State};
    use std::{
        fs::{self, File},
        io::{self, BufRead, Read, Write},
        os::{
            fd::{AsRawFd, FromRawFd, OwnedFd},
            unix::{
                net::UnixStream,
                process::{CommandExt, ExitStatusExt},
            },
        },
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        sync::{Arc, Mutex},
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

    fn supervise(root: &Path, shell: bool) -> io::Result<()> {
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
        let mut command = if shell {
            let mut command = Command::new("/bin/sh");
            command
                .args([
                    "-c",
                    r#"
printf 'before\n'
i=0
while [ ! -e "$1.release" ]; do
    i=$((i + 1)); [ "$i" -lt 500 ] || exit 2
    /bin/sleep 0.01
done
printf 'after\n'
"#,
                    "log_probe-shell",
                ])
                .arg(root);
            command
        } else {
            let mut command = Command::new(&executable);
            command.arg("--produce").arg(root);
            command
        };
        let producer = command
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
        let mut producer_group = match Group::capture(&mut producer) {
            Ok(group) => group,
            Err(error) => {
                let _ = producer.kill();
                let _ = producer.wait();
                let _ = collector.kill();
                let _ = collector.wait();
                return Err(io::Error::other(error));
            }
        };
        println!("{} {}", collector.id(), producer.id());
        io::stdout().flush()?;
        let watched_root = root.to_owned();
        let status = Arc::new(Mutex::new(InstanceStatus::default()));
        status.lock().unwrap().all_ready();
        let watched_status = Arc::clone(&status);
        let monitor = thread::spawn(move || -> io::Result<()> {
            let exit = collector.wait()?;
            let mut status = watched_status.lock().unwrap();
            status.collector_exited(exit.success());
            fs::write(
                watched_root.with_extension("status"),
                serde_json::to_vec(&*status)?,
            )?;
            fs::write(watched_root.with_extension("collector-exited"), b"exited")
        });
        // The fixture exposes only an explicit stop marker; collector failure does
        // not signal the producer, spawn a replacement, or repair its broken pipe.
        while observe_exit(&producer).map_err(io::Error::other)?.is_none() {
            if root.with_extension("stop").exists() {
                status.lock().unwrap().request_stop();
                producer_group
                    .stop(Duration::from_secs(1))
                    .map_err(io::Error::other)?;
                producer.wait()?;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        // A leader may have exited while a descendant still holds the pipes.
        wait_for(|| {
            producer_group
                .refresh()
                .is_ok_and(|members| members.is_empty())
        })?;
        let exit = producer.wait()?;
        fs::write(
            root.with_extension("exit"),
            serde_json::to_vec(&serde_json::json!({
                "code": exit.code(), "signal": exit.signal()
            }))?,
        )?;
        monitor
            .join()
            .map_err(|_| io::Error::other("Collector monitor panicked"))??;
        let mut status = status.lock().unwrap();
        status.finish_stop(
            producer_group
                .refresh()
                .map_err(io::Error::other)?
                .is_empty(),
        );
        fs::write(root.with_extension("status"), serde_json::to_vec(&*status)?)?;
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
        fs::write(result(root), &outcome)?;
        if outcome == "BrokenPipe" {
            // A real service may catch EPIPE and stay alive. Keep that case alive
            // until the supervisor receives the explicit user-stop fixture marker.
            wait_for(|| root.with_extension("stop").exists())?;
            thread::sleep(Duration::from_secs(1));
        }
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
        fixture_mode(root, false)
    }

    fn fixture_mode(root: &Path, shell: bool) -> io::Result<Fixture> {
        let mut supervisor = Command::new(std::env::current_exe()?)
            .arg(if shell {
                "--supervise-shell"
            } else {
                "--supervise"
            })
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
        let window = apphost::logs::read_window(&root, 0, u128::MAX, 1024)?;
        let stdout: Vec<u8> = window
            .chunks
            .iter()
            .filter(|chunk| chunk.stream == Stream::Stdout)
            .flat_map(|chunk| chunk.bytes.iter().copied())
            .collect();
        assert_eq!(stdout, b"before\nafter\x00\xff\n");
        assert!(
            !window.prefix_evicted
                && !window.truncated
                && !window.incomplete_tail
                && !window.sequence_gap
        );
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
        let status: serde_json::Value =
            serde_json::from_slice(&fs::read(root.with_extension("status"))?)?;
        assert_eq!(status["state"], serde_json::json!(State::Failed));
        assert_eq!(status["log_incomplete"], true);
        assert_eq!(unsafe { libc::kill(fixture.pids[1] as i32, 0) }, 0);
        assert!(fixture.supervisor.try_wait()?.is_none());
        fs::write(root.with_extension("stop"), b"explicit stop")?;
        wait_for(|| fixture.supervisor.try_wait().is_ok_and(|s| s.is_some()))?;
        let status: serde_json::Value =
            serde_json::from_slice(&fs::read(root.with_extension("status"))?)?;
        assert_eq!(status["state"], serde_json::json!(State::Stopped));
        assert_eq!(status["log_incomplete"], true);
        println!(
            "collector SIGKILL: FAILED with log gap; producer remains alive until explicit stop; STOPPED retains failure"
        );
        let root = temp.path().join("shell-collector-crash");
        let mut fixture = fixture_mode(&root, true)?;
        assert_eq!(
            unsafe { libc::kill(fixture.pids[0] as i32, libc::SIGKILL) },
            0
        );
        wait_for(|| root.with_extension("collector-exited").exists())?;
        fs::write(release(&root), b"go")?;
        wait_for(|| {
            fixture
                .supervisor
                .try_wait()
                .is_ok_and(|exit| exit.is_some())
        })?;
        let exit: serde_json::Value =
            serde_json::from_slice(&fs::read(root.with_extension("exit"))?)?;
        assert_eq!(exit["signal"], libc::SIGPIPE);
        let status: serde_json::Value =
            serde_json::from_slice(&fs::read(root.with_extension("status"))?)?;
        assert_eq!(status["state"], serde_json::json!(State::Failed));
        assert_eq!(status["log_incomplete"], true);
        println!("collector SIGKILL: shell service exits on SIGPIPE; FAILED and log gap remain");
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
            Some("--supervise") => supervise(root, false),
            Some("--supervise-shell") => supervise(root, true),
            Some("--produce") => produce(root),
            _ => Err(io::Error::other("Unknown probe mode")),
        }
    }
}

fn main() -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        probe::run()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "UNSUPPORTED_PLATFORM: this process probe requires macOS",
        ))
    }
}
