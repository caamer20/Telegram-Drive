#!/usr/bin/env python3
"""Validate the generated app manifest and loopback-only cleartext policy."""

from pathlib import Path
import sys
import xml.etree.ElementTree as ET

ANDROID = "{http://schemas.android.com/apk/res/android}"
PLAYER_ACTIVITY = "com.cameronamer.telegramdrive.nativeplayer.NativePlayerActivity"
MEDIA_LIBRARY_ACTIVITY = "com.cameronamer.telegramdrive.medialibrary.MediaLibraryActivity"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"Android manifest validation failed: {message}")


def main() -> None:
    if len(sys.argv) != 4:
        raise SystemExit(
            "usage: validate_android_manifest.py <generated-android-dir> <network-config> <variant>"
        )
    generated = Path(sys.argv[1])
    network_config = Path(sys.argv[2])
    variant = sys.argv[3]
    variant_root = generated / "app" / "build" / "intermediates" / "merged_manifests" / variant
    candidates = list(variant_root.glob("process*Manifest/AndroidManifest.xml"))
    require(len(candidates) == 1, f"expected one final merged manifest for {variant}, found {len(candidates)}")
    manifest = candidates[0]
    root = ET.parse(manifest).getroot()

    permissions = {item.get(ANDROID + "name") for item in root.findall("uses-permission")}
    require("android.permission.INTERNET" in permissions, "INTERNET permission is missing")
    uses_sdk = root.find("uses-sdk")
    require(uses_sdk is not None and uses_sdk.get(ANDROID + "minSdkVersion") == "24", "minSdk is not 24")
    application = root.find("application")
    require(application is not None, "application element is missing")
    require(
        application.get(ANDROID + "networkSecurityConfig") == "@xml/native_player_network_security_config",
        "loopback network security config is not applied to the final application",
    )
    require(application.get(ANDROID + "usesCleartextTraffic") != "true", "global cleartext traffic is enabled")
    activities = [item for item in application.findall("activity") if item.get(ANDROID + "name") == PLAYER_ACTIVITY]
    require(len(activities) == 1, "NativePlayerActivity is missing or duplicated")
    activity = activities[0]
    require(activity.get(ANDROID + "exported") == "false", "NativePlayerActivity must not be exported")
    require(activity.get(ANDROID + "supportsPictureInPicture") == "true", "PiP support is missing")
    media_activities = [
        item for item in application.findall("activity")
        if item.get(ANDROID + "name") == MEDIA_LIBRARY_ACTIVITY
    ]
    require(len(media_activities) == 1, "MediaLibraryActivity is missing or duplicated")
    require(
        media_activities[0].get(ANDROID + "exported") == "false",
        "MediaLibraryActivity must not be exported",
    )

    security = ET.parse(network_config).getroot()
    base = security.find("base-config")
    require(base is not None and base.get("cleartextTrafficPermitted") == "false", "base cleartext policy is not denied")
    domains = security.findall("domain-config")
    require(len(domains) == 1 and domains[0].get("cleartextTrafficPermitted") == "true", "loopback exception is invalid")
    names = [(item.text or "").strip() for item in domains[0].findall("domain")]
    require(names == ["127.0.0.1"], "cleartext exception is broader than IPv4 loopback")
    print(f"Validated merged manifest: {manifest}")


if __name__ == "__main__":
    main()
