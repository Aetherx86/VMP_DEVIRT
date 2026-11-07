#!/bin/bash
if [ -z "$1" ]; then
    echo "Usage: $0 <sample-name>"
    exit 1
fi

gcc samples-source/"$1".c -Wl,-rpath,\$ORIGIN,-rpath-link,./ -I./vmp_sdk -L./vmp_sdk -lVMProtectSDK64 -m64 -o binaries/"$1".bin && echo "$1 OK!"
