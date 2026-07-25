# redroid-helper

一个 Rust TUI 工具：从本地 Docker 中选择原版 `redroid/redroid:*` 镜像，添加适配容器 Android 的 Magisk，并生成新的派生镜像。

工具直接通过 Docker Engine API 与 dockerd 通信，不调用 `docker` CLI。它可以在宿主机运行，也可以通过挂载 Docker socket 在容器中运行。

## Magisk 来源

> [!IMPORTANT]
> 本项目使用的 Magisk **不是 [`topjohnwu/Magisk`](https://github.com/topjohnwu/Magisk) 发布的官方原版**。

默认使用 [`ayasa520/Magisk`](https://github.com/ayasa520/Magisk) fork 发布的 [`v30.7` 修改版](https://github.com/ayasa520/Magisk/releases/tag/v30.7)。该版本针对 Waydroid、redroid 等没有常规 boot image 安装流程的容器 Android 做了 direct-system 适配。

```text
文件:     app-debug.apk
版本:     v30.7 (30700)
提交:     9ddbd0b
SHA-256:  61467cfbbcad3754a29f53fe0e9bdef11d7c5961710d93089c8aa480cf4e126a
```

相同版本号不代表它与官方 Magisk v30.7 的二进制相同。修改版兼容问题不应提交给 Magisk 官方上游。

## 上游致谢

- [`remote-android/redroid-doc`](https://github.com/remote-android/redroid-doc)：redroid 及其派生镜像文件层方案。
- [`topjohnwu/Magisk`](https://github.com/topjohnwu/Magisk)：Magisk 原始上游。
- [`ayasa520/Magisk`](https://github.com/ayasa520/Magisk)：本项目实际使用的容器修改版 Magisk。
- [`waydroid-helper/extensions`](https://github.com/waydroid-helper/extensions/tree/master/root/magisk)：Magisk 文件布局、Android init 配置和持久化数据初始化方案。
- [`ayasa520/redroid-script`](https://github.com/ayasa520/redroid-script)：通过 Docker 派生镜像向 redroid 添加 Magisk 的参考实现。

本仓库不包含或重新发布 Magisk APK。构建派生镜像时才会下载 APK、验证摘要并把所需文件加入用户本地镜像。

## Docker 运行

宿主机不需要安装 Rust。

```bash
docker build -t redroid-helper:latest .

docker run --rm -it \
  -v /var/run/docker.sock:/var/run/docker.sock \
  redroid-helper:latest
```


TUI 按键：

- `↑` / `↓`：选择镜像
- `Tab`：切换到输出标签
- `Backspace`：编辑输出标签
- `Enter`：开始构建
- `Esc`、镜像列表中的 `q` 或 `Ctrl-C`：退出

## 非交互构建

```bash
docker run --rm -it \
  -v /var/run/docker.sock:/var/run/docker.sock \
  redroid-helper:latest \
  build \
  --base redroid/redroid:12.0.0-latest \
  --target local/redroid:12.0.0-magisk
```

## 使用本地 APK

可用 `--magisk-apk` 指定上述 ayasa520 v30.7 `app-debug.apk`；其他版本、官方 APK 或其他 fork 无法通过内置 SHA-256 校验：

```bash
doas docker run --rm -it \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v "$PWD/app-debug.apk:/apk/magisk.apk:ro" \
  redroid-helper:latest \
  --magisk-apk /apk/magisk.apk \
  build \
  --base redroid/redroid:12.0.0-latest \
  --target local/redroid:12.0.0-magisk
```

## `/data/adb/magisk` 首次初始化

1. 派生镜像中的 `bootanim.rc` 在 `post-fs-data` 阶段执行 `redroid-setup.sh`。此时 `/data` 已挂载。
2. 脚本检查 `/data/adb/magisk/.redroid-helper-<Magisk版本>` 标记。
3. 没有标记时，从只读 system 文件层复制 `/system/etc/init/magisk` 到 `/data/adb/magisk`，并设置 owner 和 mode。
4. 复制成功后创建版本标记。
5. 随后才执行 Magisk 的 `--setup-sbin` 和 `--post-fs-data`。

相关实现：

- [`assets/bootanim.rc`](assets/bootanim.rc)
- [`assets/redroid-setup.sh`](assets/redroid-setup.sh)
- [`src/magisk.rs`](src/magisk.rs)

## 许可证
[MIT License](LICENSE)