# confide

`#[confide]` is a proc-macro for config structs: generates `Default` + `Debug` with serde defaults in one attribute.

## Quick Start

```rust
use confide::confide;
use std::time::Duration;

#[confide]
pub struct MyConfig {
    #[confide(default = 8080)]
    pub port: u16,

    #[confide(default_duration = "30s")]
    pub timeout: Duration,

    #[confide(default_bytes = "1 MiB")]
    pub buffer_size: u64,

    #[confide(default)]
    pub mode: String,

    #[confide(default = "127.0.0.1".to_string(), secret)]
    pub bind_address: String,
}
```

This generates `Default`, `Debug`, and per-field `#[serde(default = ...)]` so missing keys in config files are filled in automatically.

## Macro Arguments

| Argument | Effect |
| -------- | ------ |
| *(none)* | Generates both `Default` and `Debug` |
| `no_default` | Suppresses `impl Default` |
| `no_debug` | Suppresses `impl Debug` |

Combine freely: `#[confide(no_default, no_debug)]`.

## Field Annotations

### `#[confide(default)]`

Use the type's `Default::default()`. Adds `#[serde(default)]`.

### `#[confide(default = expr)]`

Use the given Rust expression. If the expression is a bare path (e.g. `Vec::new`), serde calls it directly. Otherwise a hidden helper function is generated.

```rust
#[confide(default = vec![1, 2, 3])]
pub allowed_ports: Vec<u16>,

#[confide(default = 42)]
pub retries: u32,
```

### `#[confide(default_duration = "...")]`

Parse a humantime duration string at compile time. The field type should be `std::time::Duration`. The generated code also adds `#[serde(with = "confide::humantime_serde")]` so durations serialize as human-readable strings like `"5m"` / `"2h"`.

```rust
#[confide(default_duration = "10s")]
pub heartbeat: Duration,
```

### `#[confide(default_bytes = "...")]`

Parse a bytesize string at compile time. The field should be an integer type (`u64`, `u32`, `usize`, etc.). Serialization produces IEC-formatted strings (e.g. `"1 MiB"`); deserialization accepts both raw integers and strings like `"500 KB"` / `"2 GiB"`.

```rust
#[confide(default_bytes = "16 MiB")]
pub max_size: u64,
```

### `#[confide(secret)]`

Masks the field in `Debug` output (shows `"***"`). The field is still serialized/deserialized normally.

```rust
#[confide(default = "".to_string(), secret)]
pub api_key: String,
```

Multiple annotations can be combined: `#[confide(default_bytes = "1 MiB", secret)]`.

## Requirements

- **Named fields only.** Tuple structs and unit structs are not supported.
- **Every field needs a default annotation** unless you pass `no_default` to the macro. Without it the generated `Default` impl will be incomplete.
- The `serde` crate must be in scope (as a dependency of *your* crate) for `#[serde(...)]` attributes to resolve.
- The `confide` crate must be in your dependencies for generated code references `confide::humantime_serde` and `confide::bytesize::ByteSize` at runtime.

## License

Licensed under either of [Apache License 2.0](license-apache) or [MIT license](license-mit) at your option.
