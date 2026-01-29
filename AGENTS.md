# ZipFS - FUSE-based ZIP Filesystem

A Rust-based FUSE filesystem that allows ZIP archives to be mounted and accessed as a normal directory tree. The implementation supports reading files from compressed archives, directory listings, metadata operations, and handles corrupted or encrypted ZIP files gracefully.

## Build and Test Commands

### Build
```bash
# Build the project
cargo build

# Build with optimizations
cargo build --release

# Check code (fast, no binary)
cargo check
```

### Lint
```bash
# Check formatting (Rustfmt)
cargo fmt --check

# Run Clippy for linting
cargo clippy

# All checks at once (CI equivalent)
cargo fmt --check && cargo clippy
```

### Test
```bash
# Run all tests
cargo test

# Run tests using cargo-nextest (faster)
cargo nextest run

# Run a single test by name
cargo nextest run test_mount

# Run tests with output visible
cargo nextest run -j 1

# Run tests in the tests/ directory
cargo nextest run -p zipfs --test filesystem_test
```

## Code Style Guidelines

### Formatting
- Use `cargo fmt` to format code automatically
- Follow Rustfmt's default configuration
- Run autoformatting before committing: `cargo fmt`

### Imports
- **Alphabetical order**: Imports sorted alphabetically
- **Module imports first**: Group by module
- **Blank line separation**: Separate third-party from std library, and module from local imports
- Order: `extern crate (deprecated) → std:: → external crates → local modules`

### Naming Conventions
- **Functions**: snake_case (e.g., `get_data_path`, `lookup_`)
- **Structs/Enums/Modules**: CamelCase (e.g., `ZipFs`, `FileTree`)
- **Constants**: SCREAMING_SNAKE_CASE (e.g., `TTL`, `FUSE_ROOT_ID`)
- **Private fields**: use leading underscore (e.g., `open_files` - current code uses underscore but could be made private)
- **Variables**: camelCase (e.g., `cache_size`, `mnt`)
- **Types**: PascalCase for named types, descriptive names
- **File names**: snake_case for modules (e.g., `filesystem.rs`, `file_tree.rs`)

### Type Declarations
- Use type aliases for common complex types:
  ```rust
  type INode = u64;
  type FileHandle = u64;
  type Archive = ZipArchive<SyncFile>;
  type FuseError = libc::c_int;
  ```
- Prefer newtype pattern for domain concepts
- Use `Result<T, E>` consistently for error handling
- Use `Option<T>` for nullable/optional values

### Error Handling
- **Primary error type**: `color_eyre::Result<T>` (returns `Result<T, color_eyre::eyre::Error>`)
- **Map Rust IO errors to FUSE errors** using helper functions:
  ```rust
  fn map_io_error<E>(err: E) -> FuseError
  where
      E: Into<std::io::Error>,
    {
        let err: std::io::Error = err.into();
         err.raw_os_error().unwrap-or(libc::EIO)
    }
  ```

### Documentation
- Use module-level doc comments for file-level documentation
- Add doc comments for public functions and structs
- Use `TODO:` for known issues to fix
- Use `FIXME:` for urgent issues or workarounds
- Use `NOTE:` for important implementation details

### Code Organization
- **Main logic**: Keep core filesystem logic in `filesystem.rs`
- **Tree management**: Use `file_tree.rs` for inode/path mapping
- **Mount entry point**: Use `main.rs` for CLI argument parsing and mounting
- **Public API**: Export via `lib.rs` for library usage
- **Tests**: Add tests in `tests/` directory


## Project Notes

### Test Data Files
All test data resides in `tests/data/`:

**ZIP Archives:**
- `stored.zip`: Uncompressed file (`some/nested/file.txt`, 195 bytes, type=0)
  - Content: "some content\n" repeated 15 times (total 15 lines)
- `compressed.zip`: Deflated file (`some/nested/file.txt`, compressed 18 bytes -> actual 195 bytes, type=8)
  - Content: "some content\n" repeated 15 times across compressed data
- `encrypted.zip`: Encrypted file (`some/nested/file.txt`, 207 bytes, type=0)
  - Content: "some content\n" repeated 15 times (metadata only, file is encrypted)
- `corrupt.zip`: Corrupted archive that fails to parse gracefully
  - Content: Corrupted archive data, no valid files

**Plain Files:**
- `passthrough.txt`: Content for testing passthrough operations
  - Contains "some content\n" lines for passthrough tests


### Configuration files
- `Cargo.toml`: Package configuration and dependencies
- `flake.nix`: Nix build configuration using `devenv`
- `.github/workflows/ci.yml`: CI/CD pipeline for linting and testing

### Environment
- This project uses `devenv` for environment management
- The CI uses `cargo-nextest` for faster test execution
- Requires `fuse3` for FUSE filesystem operations

### Dependencies
- Uses `color_eyre::eyre` for error handling with `Result` type
- Uses `tracing` and `tracing-subscriber` for logging
- Uses `rstest` for property-based testing with `#[rstest]` and `#[case]`
- Uses `bimap::BiMap` for bidirectional inode/path mapping
