//! Real ENOSPC probe on a disposable 16 MiB image, never on the user's data volume.
#[cfg(target_os = "macos")]
fn run() -> std::io::Result<()> {
    use apphost::logs::{LogWriter, Stream};
    use std::{
        fs,
        io::{self, Write},
        os::unix::fs::MetadataExt,
        path::{Path, PathBuf},
        process::Command,
    };

    fn command(args: &[&std::ffi::OsStr]) -> io::Result<()> {
        let output = Command::new("/usr/bin/hdiutil").args(args).output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "hdiutil {:?}: {}\n{}\n{}",
                args,
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Ok(())
    }
    struct Volume {
        mount: PathBuf,
        temp: Option<tempfile::TempDir>,
    }
    impl Volume {
        fn detach(&mut self) -> io::Result<()> {
            command(&["detach".as_ref(), self.mount.as_os_str(), "-quiet".as_ref()])?;
            self.temp.take();
            Ok(())
        }
    }
    impl Drop for Volume {
        fn drop(&mut self) {
            if self.temp.is_some()
                && let Err(error) = self.detach()
            {
                // Never recursively remove a directory while its image is still mounted.
                let path = self.temp.take().unwrap().keep();
                eprintln!("Image cleanup failed: {error}; retained {}", path.display());
            }
        }
    }
    fn probe(mount: &Path) -> io::Result<()> {
        let mut writer = LogWriter::create(mount.join("logs"), 1024, 3)?;
        writer.append("fixture", Stream::Stdout, b"before full\n")?;
        let filler_path = mount.join("filler");
        let mut filler = fs::File::create(&filler_path)?;
        let mut full = false;
        // A finite bound is mandatory even if the image was mounted incorrectly.
        for _ in 0..512 {
            if let Err(error) = filler.write_all(&[0xa5; 65536]) {
                if error.raw_os_error() != Some(libc::ENOSPC) {
                    return Err(error);
                }
                full = true;
                break;
            }
        }
        if !full {
            return Err(io::Error::other(
                "16 MiB image did not report ENOSPC within 32 MiB",
            ));
        }
        let error = writer
            .append("fixture", Stream::Stderr, &vec![0xff; 64 * 1024])
            .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ENOSPC));
        drop(filler);
        fs::remove_file(filler_path)?;
        // Freeing space does not resurrect a writer with an unknown partial write.
        assert!(
            writer
                .append("fixture", Stream::Stdout, b"no retry")
                .is_err()
        );
        println!(
            "Real ENOSPC: log write fails explicitly; freeing space does not restart collection"
        );
        Ok(())
    }

    let temp = tempfile::tempdir()?;
    let image = temp.path().join("full.dmg");
    let mount = temp.path().join("volume");
    fs::create_dir(&mount)?;
    command(&[
        "create".as_ref(),
        "-size".as_ref(),
        "16m".as_ref(),
        "-fs".as_ref(),
        "HFS+".as_ref(),
        "-type".as_ref(),
        "UDIF".as_ref(),
        image.as_os_str(),
    ])?;
    command(&[
        "attach".as_ref(),
        image.as_os_str(),
        "-mountpoint".as_ref(),
        mount.as_os_str(),
        "-nobrowse".as_ref(),
        "-quiet".as_ref(),
    ])?;
    let mut volume = Volume {
        mount,
        temp: Some(temp),
    };
    if fs::metadata(&volume.mount)?.dev() == fs::metadata(volume.mount.parent().unwrap())?.dev() {
        return Err(io::Error::other(
            "Refusing to fill a directory on the host volume",
        ));
    }
    let result = probe(&volume.mount);
    volume.detach()?;
    result
}

fn main() -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        run()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(std::io::Error::other(
            "UNSUPPORTED_PLATFORM: disk image probe requires macOS",
        ))
    }
}
