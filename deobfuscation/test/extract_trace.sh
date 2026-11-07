#!/bin/bash

# Usage: ./extract_trace.sh start-end element_list [inputfile]

if [[ $# -lt 2 || $# -gt 3 ]]; then
    echo "Usage: $0 start-end element_list [inputfile]"
    exit 1
fi

# Parse the range
range=$1
if [[ $range =~ ^([0-9]+)-([0-9]+)$ ]]; then
    start=${BASH_REMATCH[1]}
    end=${BASH_REMATCH[2]}
else
    echo "Invalid range format. Use start-end (e.g., 1000-2000)."
    exit 1
fi

# Parse the element list
element_list=$2
IFS=',' read -ra elements_to_extract <<< "$element_list"

# Input file or stdin
if [[ $# -eq 3 ]]; then
    infile="$3"
else
    infile="/dev/stdin"
fi

# Prepare elements_to_extract array for awk
elements_string=""
for index in "${elements_to_extract[@]}"; do
    elements_string+="${index},"
done
# Remove the trailing comma
elements_string=${elements_string%,}

# Process the file
awk -v start="$start" -v end="$end" -v elements="${elements_string}" '
BEGIN {
    # Split the elements to extract into an array
    n = split(elements, elems, ",")
    for (i = 1; i <= n; i++) {
        elems[i] = elems[i] + 0  # Convert to number
        elements_to_extract[elems[i]] = 1
    }
}
{
    # Skip empty lines
    if ($0 ~ /^[[:space:]]*$/) next

    # Extract the instruction sequence number (first field)
    seq_num = $1 + 0  # Convert to number

    if (seq_num >= start && seq_num <= end) {
        # Remove the trailing double semicolons
        sub(/;;$/, "", $0)

        # Split the line into parts separated by semicolons
        n = split($0, arr, ";")

        # Extract instruction by removing the sequence number and space from arr[1]
        sub(/^[^ ]+ /, "", arr[1])
        instruction = arr[1]

        # Build the elements array
        elem_index = 1
        elements_array[elem_index] = seq_num  # Element 1: sequence number
        elem_index++
        elements_array[elem_index] = instruction  # Element 2: instruction
        elem_index++

        # Process the rest of the elements
        for (i = 2; i <= n; i++) {
            elements_array[elem_index] = arr[i]
            elem_index++
        }

        total_elements = elem_index - 1  # Total number of elements

        # Prepare the output by extracting specified elements
        output = ""
        for (i = 1; i <= total_elements; i++) {
            if (i in elements_to_extract) {
                if (output != "") {
                    output = output ";" elements_array[i]
                } else {
                    output = elements_array[i]
                }
            }
        }
        print output
    }
}
' "$infile"