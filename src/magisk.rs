use std::{
    env, fs,
    io::{Cursor, Read},
    path::Path,
};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use tar::{Builder, Header};
use zip::ZipArchive;

pub const VERSION: &str = "v30.7";
pub const APK_URL: &str =
    "https://github.com/ayasa520/Magisk/releases/download/v30.7/app-debug.apk";
pub const APK_SHA256: &str = "61467cfbbcad3754a29f53fe0e9bdef11d7c5961710d93089c8aa480cf4e126a";

const BOOTANIM_RC: &[u8] = include_bytes!("../assets/bootanim.rc");
const REDROID_SETUP_SH: &str = include_str!("../assets/redroid-setup.sh");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageArch {
    Amd64,
    Arm64,
    X86,
    Arm,
}

impl ImageArch {
    pub fn from_docker(value: &str) -> Result<Self> {
        match value {
            "amd64" | "x86_64" => Ok(Self::Amd64),
            "arm64" | "aarch64" => Ok(Self::Arm64),
            "386" | "x86" => Ok(Self::X86),
            "arm" => Ok(Self::Arm),
            other => bail!("不支持的镜像架构：{other}"),
        }
    }

    fn apk_dir(self) -> &'static str {
        match self {
            Self::Amd64 => "x86_64",
            Self::Arm64 => "arm64-v8a",
            Self::X86 => "x86",
            Self::Arm => "armeabi-v7a",
        }
    }
}

pub async fn load_apk(explicit_path: Option<&Path>) -> Result<Vec<u8>> {
    let path = explicit_path
        .map(Path::to_path_buf)
        .or_else(|| env::var_os("REDROID_HELPER_MAGISK_APK").map(Into::into));

    let bytes = if let Some(path) = path {
        fs::read(&path).with_context(|| format!("读取 Magisk APK：{}", path.display()))?
    } else {
        reqwest::get(APK_URL)
            .await
            .context("下载 Magisk APK")?
            .error_for_status()
            .context("Magisk 下载服务器返回错误")?
            .bytes()
            .await
            .context("读取 Magisk APK 下载内容")?
            .to_vec()
    };

    verify_apk(&bytes)?;
    Ok(bytes)
}

pub fn verify_apk(apk: &[u8]) -> Result<()> {
    let actual = Sha256::digest(apk)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual != APK_SHA256 {
        bail!("Magisk APK SHA-256 不匹配：期望 {APK_SHA256}，实际 {actual}");
    }
    Ok(())
}

pub fn build_layer(apk: &[u8], arch: ImageArch) -> Result<Vec<u8>> {
    let mut zip = ZipArchive::new(Cursor::new(apk)).context("解析 Magisk APK")?;
    let mut layer = Builder::new(Vec::new());

    append_dir(&mut layer, "system/etc/init/magisk", 0o755, 2000, 0)?;
    append_dir(
        &mut layer,
        "system/etc/init/magisk/chromeos",
        0o755,
        2000,
        0,
    )?;
    append_dir(&mut layer, "sbin", 0o755, 0, 0)?;

    let lib_dir = arch.apk_dir();
    for (source, target) in [
        (format!("lib/{lib_dir}/libbusybox.so"), "busybox"),
        (format!("lib/{lib_dir}/libmagisk.so"), "magisk"),
        (format!("lib/{lib_dir}/libmagiskboot.so"), "magiskboot"),
        (format!("lib/{lib_dir}/libmagiskinit.so"), "magiskinit"),
        (format!("lib/{lib_dir}/libmagiskpolicy.so"), "magiskpolicy"),
    ] {
        let data = read_zip(&mut zip, &source)?;
        append_file(
            &mut layer,
            &format!("system/etc/init/magisk/{target}"),
            &data,
            0o755,
            2000,
            0,
        )?;
    }

    for (source, target, mode) in [
        ("assets/chromeos/futility", "chromeos/futility", 0o755),
        (
            "assets/chromeos/kernel.keyblock",
            "chromeos/kernel.keyblock",
            0o755,
        ),
        (
            "assets/chromeos/kernel_data_key.vbprivk",
            "chromeos/kernel_data_key.vbprivk",
            0o755,
        ),
        ("assets/addon.d.sh", "addon.d.sh", 0o755),
        ("assets/boot_patch.sh", "boot_patch.sh", 0o755),
        ("assets/stub.apk", "stub.apk", 0o755),
        ("assets/util_functions.sh", "util_functions.sh", 0o755),
    ] {
        let data = read_zip(&mut zip, source)?;
        append_file(
            &mut layer,
            &format!("system/etc/init/magisk/{target}"),
            &data,
            mode,
            2000,
            0,
        )?;
    }

    append_file(
        &mut layer,
        "system/etc/init/magisk/magisk.apk",
        apk,
        0o755,
        2000,
        0,
    )?;
    let redroid_setup = REDROID_SETUP_SH.replace("@MAGISK_VERSION@", VERSION);
    append_file(
        &mut layer,
        "system/etc/init/magisk/redroid-setup.sh",
        redroid_setup.as_bytes(),
        0o755,
        2000,
        0,
    )?;
    append_file(
        &mut layer,
        "system/etc/init/bootanim.rc",
        BOOTANIM_RC,
        0o644,
        0,
        0,
    )?;

    layer.finish().context("完成 Magisk 文件层")?;
    layer.into_inner().context("生成 Magisk 文件层")
}

pub fn build_context(base: &str, layer: &[u8]) -> Result<Vec<u8>> {
    let dockerfile = format!(
        "FROM {base}\nADD magisk-layer.tar /\nLABEL io.github.redroid-helper.magisk-version=\"{VERSION}\" io.github.redroid-helper.base-image=\"{base}\" io.github.redroid-helper.managed=\"true\"\n"
    );
    let mut context = Builder::new(Vec::new());
    append_file(
        &mut context,
        "Dockerfile",
        dockerfile.as_bytes(),
        0o644,
        0,
        0,
    )?;
    append_file(&mut context, "magisk-layer.tar", layer, 0o644, 0, 0)?;
    context.finish().context("完成 Docker 构建上下文")?;
    context.into_inner().context("生成 Docker 构建上下文")
}

fn read_zip(zip: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>> {
    let mut file = zip
        .by_name(name)
        .with_context(|| format!("Magisk APK 缺少 {name}"))?;
    let mut data = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut data)
        .with_context(|| format!("读取 APK 内的 {name}"))?;
    Ok(data)
}

fn append_dir(tar: &mut Builder<Vec<u8>>, path: &str, mode: u32, uid: u64, gid: u64) -> Result<()> {
    let mut header = Header::new_gnu();
    header.set_entry_type(tar::EntryType::Directory);
    header.set_size(0);
    set_metadata(&mut header, mode, uid, gid);
    header.set_cksum();
    tar.append_data(&mut header, path, Cursor::new([]))?;
    Ok(())
}

fn append_file(
    tar: &mut Builder<Vec<u8>>,
    path: &str,
    data: &[u8],
    mode: u32,
    uid: u64,
    gid: u64,
) -> Result<()> {
    let mut header = Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_size(data.len() as u64);
    set_metadata(&mut header, mode, uid, gid);
    header.set_cksum();
    tar.append_data(&mut header, path, Cursor::new(data))?;
    Ok(())
}

fn set_metadata(header: &mut Header, mode: u32, uid: u64, gid: u64) {
    header.set_mode(mode);
    header.set_uid(uid);
    header.set_gid(gid);
    header.set_mtime(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_docker_architectures() {
        assert_eq!(ImageArch::from_docker("amd64").unwrap(), ImageArch::Amd64);
        assert_eq!(ImageArch::from_docker("arm64").unwrap(), ImageArch::Arm64);
        assert!(ImageArch::from_docker("mips64").is_err());
    }

    #[test]
    fn context_contains_safe_literal_base() {
        let context = build_context("redroid/redroid:16.0.0-latest", b"layer").unwrap();
        let mut archive = tar::Archive::new(Cursor::new(context));
        let dockerfile = archive
            .entries()
            .unwrap()
            .find_map(|entry| {
                let mut entry = entry.unwrap();
                (entry.path().unwrap() == Path::new("Dockerfile")).then(|| {
                    let mut text = String::new();
                    entry.read_to_string(&mut text).unwrap();
                    text
                })
            })
            .unwrap();
        assert!(dockerfile.starts_with("FROM redroid/redroid:16.0.0-latest\n"));
        assert!(dockerfile.contains("managed=\"true\""));
    }

    #[test]
    fn seeds_persistent_data_before_post_fs_data() {
        let bootanim = str::from_utf8(BOOTANIM_RC).unwrap();
        let seed_position = bootanim.find("redroid-setup.sh").unwrap();
        let post_fs_data_position = bootanim.find("--post-fs-data").unwrap();
        assert!(seed_position < post_fs_data_position);
        assert!(REDROID_SETUP_SH.contains("cp -R \"$source_dir\" \"$target_dir\""));
        assert!(REDROID_SETUP_SH.contains("@MAGISK_VERSION@"));
    }

    #[tokio::test]
    #[ignore = "需要访问 GitHub 下载约 40 MiB 的 APK"]
    async fn published_apk_contains_both_primary_architectures() {
        let apk = load_apk(None).await.unwrap();
        let amd64 = build_layer(&apk, ImageArch::Amd64).unwrap();
        let arm64 = build_layer(&apk, ImageArch::Arm64).unwrap();
        assert!(amd64.len() > apk.len());
        assert!(arm64.len() > apk.len());
    }
}
