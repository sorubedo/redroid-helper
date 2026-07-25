mod docker;
mod magisk;
mod tui;

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use tokio::sync::mpsc::unbounded_channel;

use crate::docker::{BuildRequest, Progress};

#[derive(Debug, Parser)]
#[command(version, about = "通过 Docker API 为 redroid 原版镜像集成 Magisk")]
struct Cli {
    /// 使用本地修改版 Magisk APK；也可设置 REDROID_HELPER_MAGISK_APK
    #[arg(
        long,
        global = true,
        env = "REDROID_HELPER_MAGISK_APK",
        value_name = "PATH"
    )]
    magisk_apk: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 列出 TUI 可选择的本地 redroid 原版镜像
    List,
    /// 非交互构建，便于脚本和 CI 使用
    Build {
        /// Docker 中已有的 redroid 基镜像
        #[arg(long)]
        base: String,
        /// 输出镜像标签；默认在基镜像标签后加 -magisk
        #[arg(long)]
        target: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let docker = docker::connect()?;
    docker::ping(&docker).await?;

    match cli.command {
        None => {
            let images = docker::list_redroid_images(&docker).await?;
            tui::run(docker, images, cli.magisk_apk).await
        }
        Some(Command::List) => {
            for image in docker::list_redroid_images(&docker).await? {
                println!("{}\t{}\t{}", image.reference, image.architecture, image.id);
            }
            Ok(())
        }
        Some(Command::Build { base, target }) => {
            let architecture = docker::inspect_architecture(&docker, &base).await?;
            let target = target.unwrap_or_else(|| docker::default_target(&base));
            let (sender, mut receiver) = unbounded_channel();
            docker::spawn_build(
                docker,
                BuildRequest {
                    base,
                    architecture,
                    target,
                    apk_path: cli.magisk_apk,
                },
                sender,
            );

            while let Some(event) = receiver.recv().await {
                match event {
                    Progress::Log(line) => println!("{line}"),
                    Progress::Finished(target) => {
                        println!("成功生成 {target}");
                        return Ok(());
                    }
                    Progress::Failed(message) => bail!(message),
                }
            }
            bail!("构建任务意外结束")
        }
    }
}
