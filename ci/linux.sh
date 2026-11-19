#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Run the test suite on Linux in Docker.
#
#   ci/linux.sh                                  # rust:slim
#   GLASSBOX_LINUX_IMAGES="rust:1.90-slim" ci/linux.sh
#
# The working tree is sent into the container: tracked files with any
# uncommitted edits, plus new files git does not ignore. Tests run without
# network access and as an unprivileged user (root would bypass the
# permission checks under test). Exits non-zero if any image fails.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
# macOS tar (bsdtar) adds Apple extended attributes that GNU tar warns about.
tar_flags=""
if tar --version 2>/dev/null | grep -q bsdtar; then
    tar_flags="--no-mac-metadata --no-xattrs"
fi
status=0
for image in ${GLASSBOX_LINUX_IMAGES:-rust:slim}; do
    echo "== Linux, $image"
    if (cd "$root" && git ls-files -z --cached --others --exclude-standard | tar $tar_flags --null -T - -cf -) |
        docker run --rm -i --network none --user 1000:1000 -e HOME=/tmp -e CARGO_NET_OFFLINE=true "$image" \
            sh -c 'mkdir /tmp/work && cd /tmp/work && tar -xf - && cargo test'
    then
        echo "== $image passed"
    else
        echo "== $image FAILED"
        status=1
    fi
done
exit $status
