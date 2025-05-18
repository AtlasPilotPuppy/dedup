#!/bin/bash
set -e

# Change to the script directory
cd "$(dirname "$0")"

# Clean up any existing containers
echo "Cleaning up any existing containers..."
docker-compose down -v

# Build and run the tests
echo "Building and running Docker SSH API tests..."
docker-compose up --build --abort-on-container-exit

# Clean up
echo "Cleaning up containers..."
docker-compose down -v

echo "Tests completed." 