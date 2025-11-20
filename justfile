# Voyager Golden Record Explorer - Just Commands

# Default recipe shows available commands
default:
    @just --list

# Run the app with default features (audio_playback enabled)
run:
    cargo run

# Run the app with debug logging
run-debug:
    $env:RUST_LOG="debug"; cargo run

# Run the app without audio playback
run-no-audio:
    cargo run --no-default-features

# Build in debug mode
build:
    cargo build

# Build in release mode
build-release:
    cargo build --release

# Build without audio feature
build-no-audio:
    cargo build --no-default-features

# Format code
fmt:
    cargo fmt

# Check formatting without modifying files
fmt-check:
    cargo fmt -- --check

# Run clippy with default features
clippy:
    cargo clippy --all-targets -- -D warnings

# Run clippy without audio feature
clippy-no-audio:
    cargo clippy --no-default-features --all-targets -- -D warnings

# Run clippy with audio feature
clippy-audio:
    cargo clippy --features audio_playback --all-targets -- -D warnings

# Run all clippy checks
clippy-all: clippy-no-audio clippy-audio

# Type check with default features
check:
    cargo check

# Type check without audio feature
check-no-audio:
    cargo check --no-default-features

# Type check with audio feature
check-audio:
    cargo check --features audio_playback

# Run all checks
check-all: check-no-audio check-audio

# Run tests with default features
test:
    cargo test

# Run tests without audio feature
test-no-audio:
    cargo test --no-default-features --verbose

# Run tests with audio feature
test-audio:
    cargo test --features audio_playback --verbose

# Run all tests
test-all: test-no-audio test-audio

# Run only unit tests
test-unit:
    cargo test --lib

# Run only integration tests
test-integration:
    cargo test --test integration_tests

# Run a specific test (usage: just test-one <test_name>)
test-one TEST:
    cargo test {{TEST}}

# Generate test coverage report
coverage:
    cargo tarpaulin --out html

# Build documentation and open in browser
docs:
    cargo doc --open

# Run all CI checks (excluding builds) - run this before pushing
ci: fmt-check clippy-all test-all check-all
    @echo "✓ All CI checks passed!"

# Clean build artifacts
clean:
    cargo clean

# Install git hooks
install-hooks:
    git config core.hooksPath githooks
    @echo "✓ Git hooks installed"
