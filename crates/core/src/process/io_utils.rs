use indicatif::HumanDuration;
use miette::{miette, IntoDiagnostic, Result};
use paris::Logger;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::thread::{sleep, JoinHandle};
use std::time::{Duration, Instant};
use std::{fs, thread};

#[cfg(target_os = "windows")]
pub async fn kill_process(process_name: &str) -> Result<()> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::ProcessStatus::{
        K32EnumProcessModules, K32EnumProcesses, K32GetModuleBaseNameW,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, TerminateProcess, PROCESS_QUERY_INFORMATION, PROCESS_TERMINATE,
        PROCESS_VM_READ,
    };

    let mut kill_list: Vec<u32> = vec![];
    const PROCESSES_SIZE: usize = 10240;
    let mut processes = [0u32; PROCESSES_SIZE];
    let mut process_count = 0;
    unsafe {
        K32EnumProcesses(&mut processes[0], PROCESSES_SIZE as u32, &mut process_count);
        for process_id in processes.iter().take(process_count as usize) {
            let process_handle =
                OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, *process_id);
            let mut module = 0isize;
            let mut cb_needed = 0u32;
            if K32EnumProcessModules(process_handle, &mut module, 4, &mut cb_needed) > 0 {
                let mut text: [u16; 512] = [0; 512];
                let len = K32GetModuleBaseNameW(
                    process_handle,
                    module,
                    text.as_mut_ptr(),
                    (text.len() / 4).try_into().unwrap(),
                );
                let name = String::from_utf16_lossy(&text[..len as usize]);
                if name.eq(process_name) {
                    warn!("killing process {process_id}: \"{name}\"");
                    kill_list.push(*process_id);
                }
            }

            CloseHandle(process_handle);
        }
        for id in kill_list {
            let process_handle = OpenProcess(PROCESS_TERMINATE, 0, id);
            TerminateProcess(process_handle, 0);
            CloseHandle(process_handle);
        }
    }
    Ok(())
}

pub fn symlink(original: &Path, link: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(original, link).into_diagnostic()?;
    }
    #[cfg(windows)]
    {
        let status = runas::Command::new("cmd.exe")
            .arg("/C")
            .arg("mklink")
            .arg("/D")
            .arg(link)
            .arg(original)
            .status()
            .into_diagnostic()?;
        if !status.success() {
            return Err(crate::errors::ModSymlinkFailed {}.into());
        }
    }
    Ok(())
}

pub fn extract_archive(
    archive: &str,
    target_directory: &Path,
    workspace_path: &Path,
) -> Result<()> {
    if archive.is_empty() {
        return Err(miette!("archive may not be empty"));
    }
    let workspace_data_path = workspace_path.join(PathBuf::from("data"));
    let started = Instant::now();

    #[cfg(windows)]
    {
        use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
        let file = fs::File::open(archive).into_diagnostic()?;
        info!(
            "Extracting <bright-blue>{}</> to <magenta>{}</>",
            &archive,
            target_directory.to_str().unwrap()
        );

        let mut archive = zip::ZipArchive::new(file).into_diagnostic()?;

        let mut files: Vec<String> = vec![];
        for i in 0..archive.len() {
            files.push(archive.by_index(i).into_diagnostic()?.name().into());
        }
        if workspace_data_path.exists() {
            files = files
                .into_iter()
                .filter(|file| !file.contains("/data/"))
                .collect();
        }
        let bar = ProgressBar::new(files.len() as u64);
        bar.set_draw_target(ProgressDrawTarget::stdout());
        bar.set_style(ProgressStyle::default_spinner().template("{msg}\n{wide_bar} {pos}/{len}").expect("failed to set spinner style"));
        for file in files {
            let message = format!("extracting {}", &file);
            bar.set_message(message);
            bar.tick();
            // output_path is like Factorio_0.18.36\bin\x64\factorio.exe
            let output_path = PathBuf::from(&file);
            // output_path is like bin\x64\factorio.exe
            let output_path = output_path
                .strip_prefix(output_path.components().next().unwrap())
                .into_diagnostic()?;
            // output_path is like $target_directory\bin\x64\factorio.exe
            let output_path = PathBuf::from(target_directory).join(PathBuf::from(output_path));

            if (&*file).ends_with('/') {
                fs::create_dir_all(&output_path).into_diagnostic()?;
            } else {
                if let Some(p) = output_path.parent() {
                    if !p.exists() {
                        fs::create_dir_all(&p).into_diagnostic()?;
                    }
                }

                let mut outfile = fs::File::create(&output_path).unwrap();
                let mut file = archive.by_name(&file).unwrap();
                std::io::copy(&mut file, &mut outfile).unwrap();
            }
            bar.inc(1);
        }
        if !workspace_data_path.exists() {
            let instance_data_path = target_directory.join(PathBuf::from("data"));
            fs::rename(&instance_data_path, &workspace_data_path).into_diagnostic()?;
        }
        bar.finish();
    }

    #[cfg(unix)]
    {
        use std::fs::File;
        use std::str::FromStr;
        let archive_path = PathBuf::from_str(archive).into_diagnostic()?;
        let mut logger = Logger::new();
        let extracted_path = workspace_path.join(PathBuf::from("factorio"));
        if !extracted_path.exists() {
            logger.loading(format!(
                "Uncompressing xz2 <bright-blue>{}</> to <magenta>{}</> ...",
                &archive_path.to_str().unwrap(),
                workspace_path.to_str().unwrap()
            ));
            let tar_xz = File::open(&archive_path).into_diagnostic()?;
            let tar = xz2::read::XzDecoder::new(tar_xz);
            let mut archive = tar::Archive::new(tar);
            archive
                .unpack(&workspace_path)
                .expect("failed to decompress xz");
            logger.success(format!(
                "Uncompressed tar <bright-blue>{}</> to <magenta>{}</>",
                &archive_path.to_str().unwrap(),
                workspace_path.to_str().unwrap()
            ));
        }
        if extracted_path.exists() {
            // fs::remove_dir(&target_directory).expect("failed to delete empty folder");
            fs::rename(&extracted_path, target_directory).expect("failed to rename");
            success!("Renamed {:?} to {:?}", &extracted_path, target_directory);
        } else {
            error!("Failed to find {:?}", &extracted_path);
        }
        let instance_data_path = target_directory.join(PathBuf::from("data"));
        if !workspace_data_path.exists() {
            fs::rename(&instance_data_path, &workspace_data_path).into_diagnostic()?;
        } else {
            fs::remove_dir_all(&instance_data_path).expect("failed to delete data folder");
        }
        symlink(&workspace_data_path, &instance_data_path)?;
    }
    info!(
        "Extracting took <yellow>{}</>",
        HumanDuration(started.elapsed())
    );
    Ok(())
}

pub async fn await_lock(lock_path: PathBuf, silent: bool) -> Result<()> {
    if lock_path.exists() {
        match std::fs::remove_file(&lock_path) {
            Ok(_) => {}
            Err(_) => {
                let mut logger = Logger::new();
                if !silent {
                    logger.loading("Waiting for .lock to disappear");
                }
                let started = Instant::now();
                for _ in 0..1000 {
                    sleep(Duration::from_millis(1));
                    if std::fs::remove_file(&lock_path).is_ok() {
                        break;
                    }
                }
                if !lock_path.exists() {
                    if !silent {
                        logger.success(format!(
                            "Successfully awaited .lock in <yellow>{:?}</>",
                            started.elapsed()
                        ));
                    }
                } else {
                    logger.done();
                    warn!("Factorio instance already running!");
                    #[cfg(windows)]
                    {
                        kill_process("factorio.exe").await?;
                    }
                    #[cfg(unix)]
                    {
                        return Err(crate::errors::FactorioAlreadyStarted {}.into());
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn report_child_death(mut child: Child) -> JoinHandle<ExitStatus> {
    thread::spawn(move || {
        let exit_code = child.wait().expect("failed to wait for child to end");
        if let Some(code) = exit_code.code() {
            error!("<red>server stopped</> with exit code <yellow>{}</>", code);
        } else {
            error!("<red>server stopped</> without exit code");
        }
        exit_code
    })
}
