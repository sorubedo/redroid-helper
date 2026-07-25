#!/system/bin/sh

# waydroid-helper/extensions performs this copy from the host after installing
# its system overlay:
# https://github.com/waydroid-helper/extensions/tree/master/root/magisk
# redroid uses /data as a persistent Docker volume, so seed the same files on
# first boot instead.
source_dir=/system/etc/init/magisk
target_dir=/data/adb/magisk
version_marker="$target_dir/.redroid-helper-@MAGISK_VERSION@"
su_path_module=/data/adb/modules/redroid_su_path
su_path_marker=/data/adb/.redroid-helper-su-path-v1

# Magisk chooses one writable PATH entry for its command applets. On redroid,
# /system/xbin is usually the smallest candidate, while some root apps only
# probe /system/bin/su. Add the compatibility path as a real Magisk module so
# it is part of Magic Mount and disappears with Magisk's unmount/hide flow.
# See https://github.com/ayasa520/redroid-script/issues/72
if [ ! -e "$su_path_marker" ]; then
    rm -rf "$su_path_module"
    mkdir -p "$su_path_module/system/bin"
    printf '%s\n' \
        'id=redroid_su_path' \
        'name=redroid su path compatibility' \
        'version=1.0' \
        'versionCode=1' \
        'author=redroid-helper' \
        'description=Expose Magisk su at /system/bin' \
        > "$su_path_module/module.prop"
    ln -s /sbin/su "$su_path_module/system/bin/su"
    chown -R 0:0 "$su_path_module"
    chmod 0755 "$su_path_module" "$su_path_module/system" "$su_path_module/system/bin"
    chmod 0644 "$su_path_module/module.prop"
    touch "$su_path_marker"
fi

if [ -e "$version_marker" ]; then
    exit 0
fi

mkdir -p /data/adb
rm -rf "$target_dir"
cp -R "$source_dir" "$target_dir"
chown -R 0:0 "$target_dir"
chmod -R 0755 "$target_dir"
touch "$version_marker"
