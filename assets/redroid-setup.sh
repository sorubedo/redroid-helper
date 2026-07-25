#!/system/bin/sh

# waydroid-helper/extensions performs this copy from the host after installing
# its system overlay:
# https://github.com/waydroid-helper/extensions/tree/master/root/magisk
# redroid uses /data as a persistent Docker volume, so seed the same files on
# first boot instead.
source_dir=/system/etc/init/magisk
target_dir=/data/adb/magisk
version_marker="$target_dir/.redroid-helper-@MAGISK_VERSION@"

if [ -e "$version_marker" ]; then
    exit 0
fi

mkdir -p /data/adb
rm -rf "$target_dir"
cp -R "$source_dir" "$target_dir"
chown -R 0:0 "$target_dir"
chmod -R 0755 "$target_dir"
touch "$version_marker"
