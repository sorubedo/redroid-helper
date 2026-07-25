#!/system/bin/sh

manager_apk=/system/etc/init/magisk/magisk.apk
manager_package=com.topjohnwu.magisk
manager_version_code=@MAGISK_VERSION_CODE@
install_marker=/data/adb/magisk/.redroid-helper-manager-@MAGISK_VERSION@

mark_installed() {
    touch "$install_marker"
    chmod 0600 "$install_marker"
}

# The embedded stub uses the same package name but versionCode=1. Accept the
# complete bundled manager or a newer one, while still upgrading an older app.
original_manager_is_current() {
    installed_version=$(
        pm dump "$manager_package" 2>/dev/null |
            sed -n 's/^[[:space:]]*versionCode=\([0-9][0-9]*\).*/\1/p' |
            head -n 1
    )
    case "$installed_version" in
        '' | *[!0-9]*) return 1 ;;
    esac
    [ "$installed_version" -ge "$manager_version_code" ]
}

# A previous redroid-helper image may already have installed the complete app
# without creating our marker.
if original_manager_is_current; then
    mark_installed
    exit 0
fi

# Do not restore the original package when Magisk has intentionally repackaged
# (hidden) its manager. The selected package is stored in Magisk's database.
repackaged_manager=$(
    /sbin/magisk --sqlite \
        "SELECT value FROM strings WHERE key='requester'" 2>/dev/null |
        sed -n 's/^value=//p' |
        head -n 1
)
if [ -n "$repackaged_manager" ] &&
    pm path "$repackaged_manager" >/dev/null 2>&1; then
    mark_installed
    exit 0
fi

# Run synchronously before `magisk --boot-complete`. Otherwise Magisk may
# asynchronously install its small stub first and race this full APK install.
if pm install -r -g "$manager_apk" &&
    original_manager_is_current; then
    mark_installed
    exit 0
fi

rm -f "$install_marker"
exit 1
