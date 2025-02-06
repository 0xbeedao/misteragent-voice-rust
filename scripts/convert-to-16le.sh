#!/bin/bash

if [ $# -ne 1 ]; then
    echo "Usage: $0 \"file_pattern\""
    echo "Example: $0 \"*.wav\""
    exit 1
fi

for file in $1; do
    if [ ! -f "$file" ]; then
        continue
    fi

    # Get base name without extension
    basename=$(echo "$file" | sed 's/\.[^.]*$//')
    output="${basename}-16LE.wav"

    # Skip if output file already exists
    if [ -f "$output" ]; then
        echo "Skipping $file - output file already exists"
        continue
    fi

    echo "Processing $file -> $output"
    
    # Convert to PCM_S16LE
    ffmpeg -i "$file" -acodec pcm_s16le -y "$output" 2>/dev/null

    if [ $? -eq 0 ]; then
        echo "Successfully converted $file"
    else
        echo "Error converting $file"
    fi
done