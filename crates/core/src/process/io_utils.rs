use crate::errors::*;
use indicatif::HumanDuration;
use miette::{DiagnosticResult, IntoDiagnostic};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[cfg(target_os = "windows")]
pub async fn kill_process(process_name: &str) -> DiagnosticResult<()> {
    let mut kill_list: Vec<u32> = vec![];
    process_list::for_each_process(|id, name| {
        if let Some(name) = name.to_str() {
            if name.contains(process_name) {
                info!("killing process {}: \"{}\"", id, name);
                kill_list.push(id);
            }
        }
    })
    .into_diagnostic("factorio::instance_setup::could_not_process_list::for_each_process")?;
    for id in kill_list {
        heim::process::get(id)
            .await
            .into_diagnostic("factorio::instance_setup::could_not_get_process")?
            .kill()
            .await
            .into_diagnostic("factorio::instance_setup::could_not_kill_process")?;
    }
    Ok(())
}

pub fn symlink(original: &Path, link: &Path) -> DiagnosticResult<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(original, link)
            .into_diagnostic("factorio::io::could_not_create_symlink")?;
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
            .into_diagnostic("factorio::io::could_not_create_symlink")?;
        if !status.success() {
            return Err(ModSymlinkFailed {}.into());
        }
    }
    Ok(())
}

pub fn extract_archive(
    archive: &str,
    target_directory: &Path,
    workspace_data_path: &Path,
) -> DiagnosticResult<()> {
    let started = Instant::now();

    #[cfg(windows)]
    {
        use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
        let file = fs::File::open(archive)
            .into_diagnostic("factorio::instance_setup::could_not_open_archive_path")?;
        info!(
            "Extracting <bright-blue>{}</> to <magenta>{}</>",
            &archive,
            target_directory.to_str().unwrap()
        );

        let mut archive = zip::ZipArchive::new(file)
            .into_diagnostic("factorio::instance_setup::could_not_open_zip")?;

        let mut files: Vec<String> = vec![];
        for i in 0..archive.len() {
            files.push(
                archive
                    .by_index(i)
                    .into_diagnostic("factorio::instance_setup::could_not_read_zip_entry")?
                    .name()
                    .into(),
            );
        }
        if workspace_data_path.exists() {
            files = files
                .into_iter()
                .filter(|file| !file.contains("/data/"))
                .collect();
        }
        let bar = ProgressBar::new(files.len() as u64);
        bar.set_draw_target(ProgressDrawTarget::stdout());
        bar.set_style(ProgressStyle::default_spinner().template("{msg}\n{wide_bar} {pos}/{len}"));
        for file in files {
            let message = format!("extracting {}", &file);
            bar.set_message(message);
            bar.tick();
            // output_path is like Factorio_0.18.36\bin\x64\factorio.exe
            let output_path = PathBuf::from(&file);
            // output_path is like bin\x64\factorio.exe
            let output_path = output_path
                .strip_prefix(output_path.components().next().unwrap())
                .into_diagnostic("factorio::instance_setup::strip_prefix")?;
            // output_path is like $target_directory\bin\x64\factorio.exe
            let output_path = PathBuf::from(target_directory).join(PathBuf::from(output_path));

            if (&*file).ends_with('/') {
                fs::create_dir_all(&output_path)
                    .into_diagnostic("factorio::instance_setup::could_not_create_unpack_dir")?;
            } else {
                if let Some(p) = output_path.parent() {
                    if !p.exists() {
                        fs::create_dir_all(&p)
                            .into_diagnostic("factorio::instance_setup::could_not_create_dir")?;
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
            fs::rename(&instance_data_path, &workspace_data_path)
                .into_diagnostic("factorio::instance_setup::could_not_rename_data")?;
        }
        bar.finish();
    }

    #[cfg(unix)]
    {
        let archive_path = PathBuf::from_str(archive)
            .into_diagnostic("factorio::output_parser::could_not_canonicalize")?;
        let tar_path = archive_path.with_extension("");
        if !tar_path.exists() {
            let mut logger = Logger::new();
            logger.loading(format!(
                "Uncompressing <bright-blue>{}</> to <magenta>{}</> ...",
                &archive_path.to_str().unwrap(),
                tar_path.to_str().unwrap()
            ));

            let tar_gz = File::open(&archive_path)
                .into_diagnostic("factorio::output_parser::could_not_canonicalize")?;
            let tar = xz2::read::XzDecoder::new(tar_gz);
            let mut archive = tar::Archive::new(tar);
            archive.unpack(&tar_path).expect("failed to decompress xz");
            logger.success(format!(
                "Uncompressed <bright-blue>{}</> to <magenta>{}</>",
                &archive_path.to_str().unwrap(),
                tar_path.to_str().unwrap()
            ));
        }
        let mut logger = Logger::new();
        logger.loading(format!(
            "Extracting <bright-blue>{}</> to <magenta>{}</> ...",
            &tar_path.to_str().unwrap(),
            workspace_path.to_str().unwrap()
        ));
        // FIXME: what did this do ...?
        // let mut archive = archiver_rs::Tar::open(&tar_path).unwrap();
        // archive.extract(workspace_path).expect("failed to extract");
        logger.success("Extraction finished");

        let extracted_path = workspace_path.join(PathBuf::from("factorio"));
        if extracted_path.exists() {
            std::fs::remove_dir(&target_directory).expect("failed to delete empty folder");
            std::fs::rename(&extracted_path, target_directory).expect("failed to rename");
            success!("Renamed {:?} to {:?}", &extracted_path, target_directory);
        } else {
            error!("Failed to find {:?}", &extracted_path);
        }

        let instance_data_path = target_directory.join(PathBuf::from("data"));
        if !workspace_data_path.exists() {
            fs::rename(&instance_data_path, &workspace_data_path)
                .into_diagnostic("factorio::output_parser::could_not_canonicalize")?;
        } else {
            std::fs::remove_dir_all(&instance_data_path).expect("failed to delete data folder");
        }
    }
    info!(
        "Extracting took <yellow>{}</>",
        HumanDuration(started.elapsed())
    );
    Ok(())
}
