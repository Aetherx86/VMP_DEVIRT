#!/bin/bash
# run_devirt.sh
# This script checks for the devirt.vmp.trace file and runs attack_vmp.py
# with the --trace1 and --symsize parameters.

TRACE_FILE="devirt.vmp.trace"

if [ ! -f "$TRACE_FILE" ]; then
    echo "Error: Trace file '$TRACE_FILE' not found!"
    exit 1
fi

echo "Running attack_vmp.py with trace file '$TRACE_FILE' and symsize 4..."
python3 ./attack_vmp.py --trace1 "$TRACE_FILE" --symsize 4 >> devirt.ll
