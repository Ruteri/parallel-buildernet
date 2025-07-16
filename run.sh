#!/bin/bash

# Distributed Block Builder - Build and Test Script

echo "========================================="
echo "Distributed Block Builder - Build & Test"
echo "========================================="
echo ""

# Check if cargo is installed
if ! command -v cargo &> /dev/null; then
    echo "Error: Cargo is not installed. Please install Rust first."
    echo "Visit: https://www.rust-lang.org/tools/install"
    exit 1
fi

# Parse command line arguments
case "$1" in
    "build")
        echo "Building project..."
        cargo build --release
        ;;
    "test")
        echo "Running all tests..."
        cargo test -- --nocapture
        ;;
    "test-quick")
        echo "Running tests without output..."
        cargo test
        ;;
    "test-protocol")
        echo "Running full protocol test..."
        cargo test test_full_protocol_flow -- --nocapture
        ;;
    "test-conflicts")
        echo "Running conflict detection test..."
        cargo test test_conflict_detection -- --nocapture
        ;;
    "test-routing")
        echo "Running geographic routing test..."
        cargo test test_geographic_routing -- --nocapture
        ;;
    "run")
        echo "Running main binary..."
        cargo run
        ;;
    "example")
        echo "Running network example..."
        cargo run --example start_network
        ;;
    "clean")
        echo "Cleaning build artifacts..."
        cargo clean
        ;;
    "fmt")
        echo "Formatting code..."
        cargo fmt
        ;;
    "check")
        echo "Checking code..."
        cargo check
        cargo clippy
        ;;
    *)
        echo "Usage: $0 {build|test|test-quick|test-protocol|test-conflicts|test-routing|run|example|clean|fmt|check}"
        echo ""
        echo "Commands:"
        echo "  build         - Build the project in release mode"
        echo "  test          - Run all tests with output"
        echo "  test-quick    - Run all tests without output"
        echo "  test-protocol - Run the full protocol flow test"
        echo "  test-conflicts- Run conflict detection tests"
        echo "  test-routing  - Run geographic routing tests"
        echo "  run           - Run the main binary"
        echo "  example       - Run the network example"
        echo "  clean         - Clean build artifacts"
        echo "  fmt           - Format the code"
        echo "  check         - Check code and run clippy"
        exit 1
        ;;
esac

echo ""
echo "Done!"
