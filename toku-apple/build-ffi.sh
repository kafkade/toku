#!/usr/bin/env bash
#
# Build the toku-ffi static libraries for iOS (device + simulator) and copy the
# generated C header into the TokuKit system-library module.
#
# Usage:
#   ./build-ffi.sh            # release build for device + simulator
#   PROFILE=debug ./build-ffi.sh
#
set -euo pipefail

PROFILE="${PROFILE:-release}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

DEVICE_TARGET="aarch64-apple-ios"
SIM_TARGET="aarch64-apple-ios-sim"

CARGO_PROFILE_FLAG=""
if [[ "${PROFILE}" == "release" ]]; then
  CARGO_PROFILE_FLAG="--release"
fi

echo "==> Ensuring iOS Rust targets are installed"
rustup target add "${DEVICE_TARGET}" "${SIM_TARGET}"

echo "==> Building toku-ffi for device (${DEVICE_TARGET}, ${PROFILE})"
cargo build ${CARGO_PROFILE_FLAG} -p toku-ffi --target "${DEVICE_TARGET}"

echo "==> Building toku-ffi for simulator (${SIM_TARGET}, ${PROFILE})"
cargo build ${CARGO_PROFILE_FLAG} -p toku-ffi --target "${SIM_TARGET}"

echo "==> Copying generated header into TokuKit"
cp "${REPO_ROOT}/crates/toku-ffi/toku.h" \
   "${SCRIPT_DIR}/TokuKit/Sources/CTokuFFI/toku.h"

echo
echo "Done. Static libraries:"
echo "  ${REPO_ROOT}/target/${DEVICE_TARGET}/${PROFILE}/libtoku_ffi.a"
echo "  ${REPO_ROOT}/target/${SIM_TARGET}/${PROFILE}/libtoku_ffi.a"
