#!/bin/bash

# sample_media.sh - Download sample media files to demonstrate media deduplication
# This script will create a folder structure with sample images, videos, and audio files
# to demonstrate the media deduplication features of dedup

set -e
echo "Creating sample media files for dedup demonstration..."

# Create necessary directories
mkdir -p demo/original
mkdir -p demo/similar_quality
mkdir -p demo/different_formats
mkdir -p demo/resized

cd demo

# Function to check if a command exists
command_exists() {
  command -v "$1" >/dev/null 2>&1
}

# Check if required tools are installed
echo "Checking required dependencies..."
required_tools=("curl" "ffmpeg" "convert")
missing_tools=()

for tool in "${required_tools[@]}"; do
  if ! command_exists "$tool"; then
    missing_tools+=("$tool")
  fi
done

if [ ${#missing_tools[@]} -gt 0 ]; then
  echo "Error: The following tools are required but not installed:"
  for tool in "${missing_tools[@]}"; do
    echo "  - $tool"
  done
  
  echo ""
  echo "Please install the missing dependencies:"
  echo "  - curl: Used for downloading files"
  echo "  - ffmpeg: Used for video and audio conversions"
  echo "  - convert (ImageMagick): Used for image conversions"
  echo ""
  echo "Installation instructions:"
  echo "  On macOS: brew install curl ffmpeg imagemagick"
  echo "  On Ubuntu/Debian: sudo apt install curl ffmpeg imagemagick"
  echo "  On Fedora/RHEL: sudo dnf install curl ffmpeg imagemagick"
  
  exit 1
fi

# Download sample image - using a more reliable source
echo "Downloading sample image..."
curl -s -L -o original/sample_image.jpg "https://samplelib.com/lib/preview/jpeg/sample-clouds-400x300.jpg"

# Verify the image was downloaded correctly
if [ ! -s original/sample_image.jpg ]; then
  echo "Error: Failed to download sample image. Please check your internet connection."
  
  # Try an alternative source
  echo "Trying alternative source..."
  curl -s -L -o original/sample_image.jpg "https://fastly.picsum.photos/id/237/400/300.jpg?hmac=Ja6jNt6NXbmbLoOESXIBldUkIl-mWQKl1o6jg5F-ikM"
  
  if [ ! -s original/sample_image.jpg ]; then
    echo "Failed to download sample image from alternative source. Exiting."
    exit 1
  fi
fi

# Create similar images with different qualities
echo "Creating similar images with different qualities..."
convert original/sample_image.jpg -quality 80 similar_quality/sample_image_medium.jpg
convert original/sample_image.jpg -quality 60 similar_quality/sample_image_low.jpg
convert original/sample_image.jpg -resize 50% resized/sample_image_small.jpg
convert original/sample_image.jpg -resize 200% resized/sample_image_large.jpg

# Create different formats of the same image
echo "Creating different formats of the same image..."
convert original/sample_image.jpg different_formats/sample_image.png
convert original/sample_image.jpg different_formats/sample_image.webp
convert original/sample_image.jpg different_formats/sample_image.bmp
convert original/sample_image.jpg different_formats/sample_image.tiff

# Download sample video
echo "Downloading sample video..."
curl -s -L -o original/sample_video.mp4 "https://samplelib.com/lib/preview/mp4/sample-5s.mp4"

# Verify the video was downloaded correctly
if [ ! -s original/sample_video.mp4 ]; then
  echo "Error: Failed to download sample video. Please check your internet connection."
  
  # Try to create a simple video using ffmpeg
  echo "Creating a simple video using ffmpeg..."
  ffmpeg -y -loglevel error -f lavfi -i testsrc=duration=5:size=320x240:rate=30 original/sample_video.mp4
  
  if [ ! -s original/sample_video.mp4 ]; then
    echo "Failed to create sample video. Exiting."
    exit 1
  fi
fi

# Create similar videos with different qualities
echo "Creating similar videos with different qualities..."
ffmpeg -y -loglevel error -i original/sample_video.mp4 -c:v libx264 -crf 23 -preset medium similar_quality/sample_video_medium.mp4
ffmpeg -y -loglevel error -i original/sample_video.mp4 -c:v libx264 -crf 28 -preset medium similar_quality/sample_video_low.mp4
ffmpeg -y -loglevel error -i original/sample_video.mp4 -vf "scale=iw/2:ih/2" resized/sample_video_small.mp4
ffmpeg -y -loglevel error -i original/sample_video.mp4 -c:v libx264 -crf 18 -preset slow different_formats/sample_video_high.mp4

# Create different formats of the same video
echo "Creating different formats of the same video..."
ffmpeg -y -loglevel error -i original/sample_video.mp4 different_formats/sample_video.webm
ffmpeg -y -loglevel error -i original/sample_video.mp4 different_formats/sample_video.mkv
ffmpeg -y -loglevel error -i original/sample_video.mp4 different_formats/sample_video.mov

# Download sample audio
echo "Downloading sample audio..."
curl -s -L -o original/sample_audio.mp3 "https://samplelib.com/lib/preview/mp3/sample-3s.mp3"

# Verify the audio was downloaded correctly
if [ ! -s original/sample_audio.mp3 ]; then
  echo "Error: Failed to download sample audio. Please check your internet connection."
  
  # Try to create a simple audio file using ffmpeg
  echo "Creating a simple audio file using ffmpeg..."
  ffmpeg -y -loglevel error -f lavfi -i "sine=frequency=440:duration=3" original/sample_audio.mp3
  
  if [ ! -s original/sample_audio.mp3 ]; then
    echo "Failed to create sample audio. Exiting."
    exit 1
  fi
fi

# Create similar audio with different qualities
echo "Creating similar audio with different qualities..."
ffmpeg -y -loglevel error -i original/sample_audio.mp3 -codec:a libmp3lame -qscale:a 2 similar_quality/sample_audio_high.mp3
ffmpeg -y -loglevel error -i original/sample_audio.mp3 -codec:a libmp3lame -qscale:a 5 similar_quality/sample_audio_medium.mp3
ffmpeg -y -loglevel error -i original/sample_audio.mp3 -codec:a libmp3lame -qscale:a 9 similar_quality/sample_audio_low.mp3

# Create different formats of the same audio
echo "Creating different formats of the same audio..."
ffmpeg -y -loglevel error -i original/sample_audio.mp3 different_formats/sample_audio.ogg
ffmpeg -y -loglevel error -i original/sample_audio.mp3 different_formats/sample_audio.wav
ffmpeg -y -loglevel error -i original/sample_audio.mp3 different_formats/sample_audio.aac
ffmpeg -y -loglevel error -i original/sample_audio.mp3 different_formats/sample_audio.flac

echo ""
echo "Sample media files created successfully!"
echo "Directory structure:"
echo "demo/"
echo "├── original             # Original media files"
echo "├── similar_quality      # Same media with different quality levels"
echo "├── different_formats    # Same media in different file formats"
echo "└── resized              # Same media with different resolutions"
echo ""
echo "To test media deduplication run:"
echo "dedup -i demo --media-mode"
echo "dedup_tui -i demo --media-mode"
echo "or"
echo "dedup_tui --dry-run demo --media-mode --media-resolution highest --media-formats png,jpg,mp4"
echo "" 