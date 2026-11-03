#!/usr/bin/env bash
# Resolves ANDROID_NDK_HOME for the current machine, installing the NDK when the
# SDK has none. Source it: `. tools/android_env.sh`
#
# Used by CI and by tools/package_android.sh.

resolve_ndk() {
  local sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
  if [ -z "$sdk" ] || [ ! -d "$sdk" ]; then
    echo "error: ANDROID_HOME is not set to an Android SDK" >&2
    return 1
  fi

  if [ -n "${ANDROID_NDK_HOME:-}" ] && [ -d "$ANDROID_NDK_HOME" ]; then
    echo "$ANDROID_NDK_HOME"
    return 0
  fi

  local found
  found="$(ls -d "$sdk"/ndk/* 2>/dev/null | sort -V | tail -1)"
  if [ -n "$found" ]; then
    echo "$found"
    return 0
  fi

  local sdkmanager
  sdkmanager="$(ls -d "$sdk"/cmdline-tools/*/bin/sdkmanager 2>/dev/null | sort -V | tail -1)"
  if [ -z "$sdkmanager" ]; then
    echo "error: no NDK and no sdkmanager found in $sdk" >&2
    return 1
  fi

  echo "Installing the Android NDK (first run only)…" >&2
  yes | "$sdkmanager" --install "ndk;27.2.12479018" >/dev/null 2>&1 || true
  if [ -d "$sdk/ndk/27.2.12479018" ]; then
    echo "$sdk/ndk/27.2.12479018"
  else
    echo "error: NDK installation failed" >&2
    return 1
  fi
}

ANDROID_NDK_HOME="$(resolve_ndk)" || exit 1
export ANDROID_NDK_HOME
