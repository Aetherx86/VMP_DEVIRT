#!/bin/bash

# Usage: ./find_regions.sh start-end [inputfile]

# Check if the correct number of arguments is provided
if [[ $# -lt 1 || $# -gt 2 ]]; then
    echo "Usage: $0 start-end [inputfile]"
    exit 1
fi

# Parse the range argument
range=$1

# Check if the range is in hexadecimal or decimal format
if [[ $range =~ ^0x[0-9a-fA-F]+-0x[0-9a-fA-F]+$ ]]; then
    # Hexadecimal range
    start_hex=${range%-*}
    end_hex=${range#*-}
    # Remove '0x' prefix
    start_hex="${start_hex/#0x/}"
    end_hex="${end_hex/#0x/}"
    # Pass as hex strings
    start="$start_hex"
    end="$end_hex"
    hex_range=1
elif [[ $range =~ ^[0-9]+-[0-9]+$ ]]; then
    # Decimal range
    start=${range%-*}
    end=${range#*-}
    hex_range=0
else
    echo "Invalid range format. Use start-end (e.g., 1000-2000 or 0x1000-0x2000)."
    exit 1
fi

# Input file or stdin
if [[ $# -eq 2 ]]; then
    infile="$2"
else
    infile="/dev/stdin"
fi

# Process the input with awk
awk -v start="$start" -v end="$end" -v hex_range="$hex_range" '
function hex2dec(s,    n, i, c, val) {
    n = 0
    for (i = 1; i <= length(s); i++) {
        c = substr(s, i, 1)
        if (c >= "0" && c <= "9")
            val = c - "0"
        else if (c >= "A" && c <= "F")
            val = ord_hex(c)
        else if (c >= "a" && c <= "f")
            val = ord_hex(c)
        else
            return -1  # invalid character
        n = n * 16 + val
    }
    return n
}
function ord_hex(c) {
    if (c == "a" || c == "A") return 10
    else if (c == "b" || c == "B") return 11
    else if (c == "c" || c == "C") return 12
    else if (c == "d" || c == "D") return 13
    else if (c == "e" || c == "E") return 14
    else if (c == "f" || c == "F") return 15
    else return -1
}
BEGIN {
    # Convert start and end to numbers
    if (hex_range) {
        start = hex2dec(start)
        end = hex2dec(end)
    } else {
        start += 0
        end += 0
    }
}
{
    # Skip empty lines
    if ($0 ~ /^[[:space:]]*$/) next

    # Remove trailing semicolons
    sub(/;*$/, "", $0)

    # Split the line into elements
    n = split($0, elements, ";")
    if (n < 2) next

    found_in_range = 0
    all_zero = 1
    for (i = 2; i <= n; i++) {
        value = elements[i]
        # Remove leading '0x' or '0X' if present
        gsub(/^(0x|0X)/, "", value)
        # Check if value is hex or decimal
        if (value ~ /^[0-9a-fA-F]+$/) {
            dec_value = hex2dec(value)
            if (dec_value == -1) continue  # invalid hex value
        } else if (value ~ /^[0-9]+$/) {
            dec_value = value + 0
        } else {
            # Skip invalid values
            continue
        }
        # Check if dec_value is within range
        if (dec_value >= start && dec_value <= end) {
            found_in_range = 1
            break
        }
        # Check if the value is non-zero
        if (dec_value != 0) {
            all_zero = 0
        }
    }
    if (!found_in_range && !all_zero) {
        print $0
    }
}
' "$infile"