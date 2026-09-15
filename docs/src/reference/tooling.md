# Tooling Reference

Rock applications use one project command: `rock`. Run it from the directory containing `rock.toml` so it can resolve the entry source, path dependencies, standard library, and build outputs.

## Create a project

Create a project directory containing this `rock.toml`:

```toml
[crate]
name = "rock_book_project"
version = "0.1.0"

[lib]
path = "main.rk"
```

Create `main.rk` beside it:

```rock
main = !->
    "rock-book-project".println!
```

The manifest's `[lib].path` selects the source entry even when that source contains an executable `main` function.

## Everyday commands

```console
$ rock format
$ rock build
$ rock run -- 7
$ rock expand
$ rock artifact
```

`format` rewrites the configured source. Inspect formatter changes before committing them, then compile again.

`build` resolves the manifest and path dependencies, then writes products below the project's `build/` directory. It prints the linked executable path on success.

`run` builds and executes the project. Values after `--` are application arguments rather than options to the build tool.

`expand` prints macro-expanded source. It is useful when a diagnostic points into generated declarations or when a repetition does not produce the expected items.

`artifact` materializes a reusable crate artifact. Normal path-dependent builds create and reuse dependency artifacts automatically, so most applications do not need to invoke it directly.

There is no `rock test` command. Use a complete Rock executable as an application smoke test, or use the Rust integration suite when changing the compiler itself.

## Standard library selection

The active toolchain supplies a matching precompiled standard library. `rock` loads it automatically for ordinary projects and makes the prelude available. A package can opt out explicitly:

```toml
[crate]
name = "bare"
version = "0.1.0"
no_std = true

[lib]
path = "lib.rk"
```

Without the standard library, familiar operators, collections, strings, and prelude traits are unavailable unless the package supplies alternatives through explicit dependencies.

## `rockup`

`rockup` installs and selects local Rock toolchains. A source checkout can be packaged and installed with:

```console
$ cargo build --release
$ target/release/rockup dev stdlib package \
    --path stdlib \
    --sysroot target/release
$ target/release/rockup install dev --path target/release
$ target/release/rockup default dev
$ target/release/rockup list
```

The first installed toolchain becomes the default automatically; `rockup default` switches it explicitly. Toolchain shims keep the user workflow on `rock` while selecting matching compiler and standard-library components behind the project command.

## Language server

`rock-lsp` communicates over standard input and output. Configure an LSP client to launch it for `.rk` files with language identifier `rock`; it is not a replacement for the application command `rock`.

| Capability | Current behavior |
| --- | --- |
| Diagnostics | Refresh on open, change, and save using source frontend analysis; messages include available codes, notes, help, and related locations. |
| Hover | Inferred variable and expression types, function signatures, and supported type, field, variant, and generic symbols. |
| Signature help | Call signature and active parameter information; a comma is the automatic trigger. Clients can also request it explicitly. |
| Project discovery | Nearest ancestor `rock.toml`; uses its entry graph, path dependency artifacts, and toolchain standard library. |
| Unsaved source | Open buffers overlay files in the entry graph; unopened files come from disk. |

The server does not advertise completion, go-to-definition, references, rename, code actions, semantic tokens, or document formatting. Use `rock format` for the configured entry file and `rock build` for full validation, including borrow checking and linking.

Diagnostics are currently published to open documents, not as a complete workspace problem index. Hover and signature help require a successful analysis snapshot; during an invalid edit they may use the last successful snapshot and can be stale. Project resolution is cached until a Rock document is saved or the server restarts; there is no manifest/dependency file-watcher refresh. Save dependency changes and restart if feedback remains out of date.

The server accepts these explicit overrides:

| Argument | Meaning |
| --- | --- |
| `--extern-artifact name=/absolute/path/to/crate.rkca` | Add an artifact or override a discovered artifact with the same crate name; repeat for multiple crates. |
| `--no-prelude` | Disable dependency-provided prelude injection. This does not disable project dependency resolution. |

These options are for standalone or compiler-development workflows, not normal application setup. Outside a project, the server uses the opened file as its entry and does not discover a standard library automatically. Within a project, `[crate].no_std = true` also disables automatic standard-library/prelude use.

## Neovim plugin

The plugin lives in [rock.nvim](https://github.com/rock-lang-org/rock.nvim). It requires Neovim 0.11 or newer and uses `vim.lsp.config` and `vim.lsp.enable`, without an additional Lua dependency. Clone that repository, then add its root in `init.lua` (replace the path with your checkout):

```lua
vim.opt.runtimepath:prepend("/absolute/path/to/rock.nvim")
require("rock").setup()
```

The plugin registers `.rk` as filetype `rock`. Its client root markers are `rock.toml` and then `.git`, and standalone buffers are allowed; a Git root alone does not supply project dependencies.

| Setup option | Meaning |
| --- | --- |
| `cmd` | Server command string or argument list; overrides automatic executable selection. |
| `extern_artifacts` | Table mapping crate names to artifact paths; becomes repeated `--extern-artifact` arguments. |
| `no_prelude` | Pass `--no-prelude`; defaults to `false`. |
| `autostart` | Enable the LSP configuration during setup; defaults to `true`. |
| `server` | Additional native `vim.lsp.Config` fields; `server.cmd` is used only when top-level `cmd` is absent. |

The default server is `rock-lsp` from `PATH`. To use a compiler-development build instead, set `cmd` to its absolute executable path.

Use `:checkhealth rock` to check prerequisites, `:RockLspInfo` to inspect the resolved configuration and clients, and `:RockLspRestart` to restart Rock clients. See [Editors and Diagnostics](../getting-started/editor-and-diagnostics.md) for local-checkout setup and explicit hover/diagnostic commands.

## Reading diagnostics

Source diagnostics carry a primary location and may include secondary labels in other files. Available diagnostic families include `parser`, `resolve`, `type`, `borrow`, `artifact`, and `toolchain`; these classify the failure rather than identify a unique error number. Notes explain context, while help messages suggest a next step. Neither is present on every error.

File, project, artifact, and toolchain failures need not have a source underline. When source text is unavailable, the terminal fallback can show a path and byte range instead of a source excerpt. In the editor, non-source project or toolchain messages can appear at the start of a buffer; that position is not evidence of an error on its first line. Use the message and named path to decide whether to fix source or setup.

The [diagnostics walkthrough](../getting-started/editor-and-diagnostics.md#reading-a-source-diagnostic) includes deliberately incorrect programs and their corrections. There is no `rock` JSON-diagnostic flag documented by the current project CLI.

## Tree-sitter grammar

Clone the separate [tree-sitter-rock](https://github.com/rock-lang-org/tree-sitter-rock) repository to develop the grammar:

```console
$ git clone https://github.com/rock-lang-org/tree-sitter-rock.git
$ cd tree-sitter-rock
$ tree-sitter generate
$ tree-sitter build
$ tree-sitter test
```

It contains highlight and indentation queries for editor integrations. Tree-sitter installation is separate from the native LSP plugin: `require("rock").setup()` does not install a parser or enable Tree-sitter highlighting. Syntax highlighting does not perform type or borrow checking. There is still no package-registry workflow.
