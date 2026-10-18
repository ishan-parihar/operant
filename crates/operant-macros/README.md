# operant-macros

Procedural macros for Operant, currently the `Configurable` derive for config structs.

## Purpose

A proc-macro crate (`[lib] proc-macro = true`). The `#[proc_macro_derive]` in `src/lib.rs` generates secret and property methods for config structs. Its doc comment lists the supported attributes.

## Configuration

No Cargo features. Attributes are set on the struct and its fields:

- `#[secret]` on a `String`, `Option<String>`, or `Vec<String>` field. A `Vec<String>` is encrypted element-wise.
- `#[nested]` on a nested struct or `Option<StructWithSecrets>` field, delegating to the child.
- `#[prefix = "..."]` on the struct, setting the dotted path prefix.

## Extension points

- Add a new attribute or generated method in `src/lib.rs`. Generated methods include `secret_fields(&self) -> Vec<SecretFieldInfo>`.
- Built with `syn` 2, `quote`, and `proc-macro2` (`Cargo.toml`).

## Limitations

- Generated code refers to `crate::config::*` and `crate::security::*`, so the consuming crate must provide those paths. `operant-config` does so with shim modules in its `src/lib.rs`.
- The crate has no crate-level doc comment; the derive docs in `src/lib.rs` are the only reference.

## Model experience

None. This is a compile-time macro for configuration types with no runtime or model-facing behaviour.
