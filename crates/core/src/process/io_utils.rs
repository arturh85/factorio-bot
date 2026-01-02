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
    use windows_sys::Win32::Foundation::{CloseHandle, HMODULE};
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
            let mut module: HMODULE = std::ptr::null_mut();
            let mut cb_needed = 0u32;
            if K32EnumProcessModules(
                process_handle,
                &mut module,
                std::mem::size_of::<HMODULE>() as u32,
                &mut cb_needed,
            ) > 0
            {
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

    // Route to appropriate extraction method based on file extension
    #[cfg(target_os = "macos")]
    if archive.ends_with(".dmg") {
        return extract_dmg(archive, target_directory, workspace_path);
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
        bar.set_style(
            ProgressStyle::default_spinner()
                .template("{msg}\n{wide_bar} {pos}/{len}")
                .expect("failed to set spinner style"),
        );
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
                .unpack(workspace_path)
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

/// Returns the path to the Factorio binary within an instance directory.
/// Handles different directory structures:
/// - macOS .app bundle: `MacOS/factorio`
/// - Windows: `bin/x64/factorio.exe`
/// - Linux: `bin/x64/factorio`
pub fn get_factorio_binary_path(instance_path: &Path) -> PathBuf {
    // Check for macOS .app bundle structure first
    let macos_binary = instance_path.join("MacOS/factorio");
    if macos_binary.exists() {
        return macos_binary;
    }

    // Check for Windows binary
    let windows_binary = instance_path.join("bin/x64/factorio.exe");
    if windows_binary.exists() {
        return windows_binary;
    }

    // Default to Linux binary path
    instance_path.join("bin/x64/factorio")
}

/// Returns the path to the data directory within an instance directory.
/// Handles different directory structures:
/// - macOS .app bundle: `data/` (extracted from Contents/data)
/// - Windows/Linux: `data/`
pub fn get_factorio_data_path(instance_path: &Path) -> PathBuf {
    instance_path.join("data")
}

/// Extracts a macOS .dmg file to the target directory.
/// Mounts the DMG, copies the app bundle contents, and unmounts.
#[cfg(target_os = "macos")]
pub fn extract_dmg(dmg_path: &str, target_directory: &Path, workspace_path: &Path) -> Result<()> {
    use std::process::Command;

    let workspace_data_path = workspace_path.join(PathBuf::from("data"));
    let started = Instant::now();
    let mut logger = Logger::new();

    logger.loading(format!("Mounting DMG <bright-blue>{}</> ...", dmg_path));

    // Mount the DMG
    let mount_output = Command::new("hdiutil")
        .args(["attach", dmg_path, "-nobrowse", "-quiet", "-plist"])
        .output()
        .into_diagnostic()?;

    if !mount_output.status.success() {
        return Err(miette!(
            "Failed to mount DMG: {}",
            String::from_utf8_lossy(&mount_output.stderr)
        ));
    }

    // Parse plist output to find mount point
    let plist_str = String::from_utf8_lossy(&mount_output.stdout);
    let mount_point = parse_dmg_mount_point(&plist_str)?;

    logger.success(format!("Mounted DMG at <bright-blue>{}</>", &mount_point));

    // Find the .app bundle
    let app_path = PathBuf::from(&mount_point).join("factorio.app/Contents");
    if !app_path.exists() {
        // Unmount before returning error
        let _ = Command::new("hdiutil")
            .args(["detach", &mount_point, "-quiet"])
            .output();
        return Err(miette!(
            "Could not find factorio.app/Contents in mounted DMG at {:?}",
            mount_point
        ));
    }

    logger.loading(format!(
        "Copying app contents to <magenta>{}</> ...",
        target_directory.to_str().unwrap()
    ));

    // Copy Contents/* to target directory using ditto (preserves permissions and attributes)
    let copy_output = Command::new("ditto")
        .args([
            app_path.to_str().unwrap(),
            target_directory.to_str().unwrap(),
        ])
        .output()
        .into_diagnostic()?;

    if !copy_output.status.success() {
        // Unmount before returning error
        let _ = Command::new("hdiutil")
            .args(["detach", &mount_point, "-quiet"])
            .output();
        return Err(miette!(
            "Failed to copy app contents: {}",
            String::from_utf8_lossy(&copy_output.stderr)
        ));
    }

    // Remove quarantine attribute to fix "app is damaged" error on macOS
    let _ = Command::new("xattr")
        .args(["-cr", target_directory.to_str().unwrap()])
        .output();

    // Re-sign the binary with ad-hoc signature to fix "killed" due to invalid signature
    // (signature becomes invalid when extracting Contents/* out of .app bundle)
    let binary_path = target_directory.join("MacOS/factorio");
    let _ = Command::new("codesign")
        .args([
            "--force",
            "--deep",
            "--sign",
            "-",
            binary_path.to_str().unwrap(),
        ])
        .output();

    // Unmount the DMG
    let _ = Command::new("hdiutil")
        .args(["detach", &mount_point, "-quiet"])
        .output();

    // Handle the data directory - move to workspace if not already there
    let instance_data_path = target_directory.join(PathBuf::from("data"));
    if instance_data_path.exists() {
        if !workspace_data_path.exists() {
            fs::rename(&instance_data_path, &workspace_data_path).into_diagnostic()?;
            symlink(&workspace_data_path, &instance_data_path)?;
            info!("Moved data directory to workspace and created symlink");
        } else {
            fs::remove_dir_all(&instance_data_path).into_diagnostic()?;
            symlink(&workspace_data_path, &instance_data_path)?;
            info!("Created symlink to existing workspace data directory");
        }
    }

    // macOS Factorio looks for data at workspace/Contents/data (relative to parent of instance)
    // Create this symlink so Factorio can find its data
    let workspace_contents_path = workspace_path.join("Contents");
    if !workspace_contents_path.exists() {
        fs::create_dir_all(&workspace_contents_path).into_diagnostic()?;
    }
    let workspace_contents_data_path = workspace_contents_path.join("data");
    if !workspace_contents_data_path.exists() {
        symlink(&workspace_data_path, &workspace_contents_data_path)?;
        info!("Created workspace/Contents/data symlink for macOS Factorio");
    }

    logger.success(format!(
        "Extracted DMG in <yellow>{}</>",
        HumanDuration(started.elapsed())
    ));

    Ok(())
}

/// Parses the plist output from hdiutil attach to find the mount point
#[cfg(target_os = "macos")]
fn parse_dmg_mount_point(plist_str: &str) -> Result<String> {
    // Simple parsing - look for mount-point in the plist output
    // The format is: <key>mount-point</key><string>/Volumes/Factorio</string>
    if let Some(start) = plist_str.find("<key>mount-point</key>") {
        let after_key = &plist_str[start..];
        if let Some(string_start) = after_key.find("<string>") {
            let value_start = string_start + 8; // length of "<string>"
            if let Some(string_end) = after_key[value_start..].find("</string>") {
                return Ok(after_key[value_start..value_start + string_end].to_string());
            }
        }
    }

    // Fallback: search /Volumes for any Factorio-like mount point
    // This handles cases like "/Volumes/Factorio 1" when already mounted
    if let Ok(entries) = fs::read_dir("/Volumes") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("Factorio") || name.starts_with("factorio") {
                let path = entry.path().to_string_lossy().to_string();
                info!("Found Factorio mount point via fallback: {}", path);
                return Ok(path);
            }
        }
    }

    Err(miette!(
        "Could not determine DMG mount point from hdiutil output:\n{}",
        plist_str
    ))
}
