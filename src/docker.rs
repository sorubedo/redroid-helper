use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use bollard::{
    Docker, body_full,
    query_parameters::{BuildImageOptionsBuilder, ListImagesOptionsBuilder},
};
use futures_util::StreamExt;
use tokio::sync::mpsc::UnboundedSender;

use crate::magisk::{self, ImageArch};

#[derive(Debug, Clone)]
pub struct ImageChoice {
    pub reference: String,
    pub id: String,
    pub architecture: String,
    pub size: i64,
}

#[derive(Debug, Clone)]
pub struct BuildRequest {
    pub base: String,
    pub architecture: String,
    pub target: String,
    pub apk_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub enum Progress {
    Log(String),
    Finished(String),
    Failed(String),
}

pub fn connect() -> Result<Docker> {
    Docker::connect_with_defaults().context(
        "无法连接 Docker。请确认 dockerd 正在运行、DOCKER_HOST 正确，或已挂载 /var/run/docker.sock",
    )
}

pub async fn ping(docker: &Docker) -> Result<()> {
    docker
        .ping()
        .await
        .context("Docker daemon 不可访问（检查 socket 权限）")?;
    Ok(())
}

pub async fn list_redroid_images(docker: &Docker) -> Result<Vec<ImageChoice>> {
    let options = ListImagesOptionsBuilder::default().all(false).build();
    let summaries = docker
        .list_images(Some(options))
        .await
        .context("列出 Docker 镜像")?;
    let mut result = Vec::new();

    for summary in summaries {
        let managed = summary
            .labels
            .get("io.github.redroid-helper.managed")
            .is_some_and(|value| value == "true");
        if managed {
            continue;
        }

        let tags: Vec<_> = summary
            .repo_tags
            .iter()
            .filter(|tag| tag.starts_with("redroid/redroid:") && !tag.contains("magisk"))
            .cloned()
            .collect();
        if tags.is_empty() {
            continue;
        }

        let inspect = docker
            .inspect_image(&summary.id)
            .await
            .with_context(|| format!("检查镜像 {}", summary.id))?;
        let architecture = inspect.architecture.unwrap_or_else(|| "unknown".into());

        for reference in tags {
            result.push(ImageChoice {
                reference,
                id: summary.id.clone(),
                architecture: architecture.clone(),
                size: summary.size,
            });
        }
    }

    result.sort_by(|a, b| a.reference.cmp(&b.reference));
    Ok(result)
}

pub async fn inspect_architecture(docker: &Docker, image: &str) -> Result<String> {
    validate_image_reference(image)?;
    let inspect = docker
        .inspect_image(image)
        .await
        .with_context(|| format!("Docker 中不存在基镜像 {image}"))?;
    inspect
        .architecture
        .context("基镜像缺少 architecture 元数据")
}

pub fn validate_image_reference(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 255
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/' | b'@')
        })
    {
        bail!("无效的 Docker 镜像引用：{value:?}");
    }
    Ok(())
}

pub fn default_target(base: &str) -> String {
    let without_digest = base.split('@').next().unwrap_or(base);
    let (repository, tag) = match without_digest.rsplit_once(':') {
        Some((repository, tag)) if !tag.contains('/') => (repository, tag),
        _ => (without_digest, "latest"),
    };
    format!("{repository}:{tag}-magisk")
}

pub async fn build_image(
    docker: Docker,
    request: BuildRequest,
    progress: UnboundedSender<Progress>,
) -> Result<()> {
    let send_log = |message: &str| {
        let _ = progress.send(Progress::Log(message.to_string()));
    };

    validate_image_reference(&request.base)?;
    validate_image_reference(&request.target)?;
    if request.base == request.target {
        bail!("输出标签不能覆盖基镜像标签");
    }

    let arch = ImageArch::from_docker(&request.architecture)?;
    send_log(&format!(
        "基镜像：{} ({})",
        request.base, request.architecture
    ));
    send_log(&format!("准备 Magisk {}…", magisk::VERSION));
    let apk = magisk::load_apk(request.apk_path.as_deref()).await?;
    send_log("APK SHA-256 校验通过");

    let layer = magisk::build_layer(&apk, arch)?;
    send_log(&format!(
        "Magisk 文件层已生成（{} MiB）",
        layer.len() / 1024 / 1024
    ));
    let context = magisk::build_context(&request.base, &layer)?;

    let options = BuildImageOptionsBuilder::default()
        .dockerfile("Dockerfile")
        .t(&request.target)
        .pull("false")
        .rm(true)
        .forcerm(true)
        .build();
    send_log("已提交构建上下文到 Docker daemon");

    let mut stream = docker.build_image(options, None, Some(body_full(context.into())));
    while let Some(item) = stream.next().await {
        let info = item.context("Docker 构建 API")?;
        if let Some(error) = info.error_detail {
            bail!(
                "Docker 构建失败：{}",
                error.message.as_deref().unwrap_or("未知错误").trim()
            );
        }
        if let Some(output) = info.stream {
            for line in output.lines().filter(|line| !line.trim().is_empty()) {
                send_log(line.trim_end());
            }
        } else if let Some(status) = info.status {
            send_log(status.trim_end());
        }
    }

    let _ = progress.send(Progress::Finished(request.target));
    Ok(())
}

pub fn spawn_build(docker: Docker, request: BuildRequest, progress: UnboundedSender<Progress>) {
    tokio::spawn(async move {
        if let Err(error) = build_image(docker, request, progress.clone()).await {
            let _ = progress.send(Progress::Failed(format!("{error:#}")));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_output_tag() {
        assert_eq!(
            default_target("redroid/redroid:16.0.0-latest"),
            "redroid/redroid:16.0.0-latest-magisk"
        );
        assert_eq!(
            default_target("local/redroid"),
            "local/redroid:latest-magisk"
        );
    }

    #[test]
    fn rejects_dockerfile_injection() {
        assert!(validate_image_reference("redroid/redroid:16.0.0-latest").is_ok());
        assert!(validate_image_reference("redroid:latest\nRUN evil").is_err());
        assert!(validate_image_reference("").is_err());
    }
}
