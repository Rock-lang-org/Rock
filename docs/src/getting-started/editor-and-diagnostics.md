# Editors and Diagnostics

An editor can show a problem while you type; a build checks whether the saved program can become an executable. This chapter sets up that feedback loop and shows how to read an error without guessing which line to change.

## Start with a project

Complete [Installation](installation.md), then create a directory with these two files. Every Rock example in this chapter is a complete replacement for `main.rk`, not an addition to the previous example.

`rock.toml`:

```toml
[crate]
name = "diagnostics_demo"
version = "0.1.0"

[lib]
path = "main.rk"
```

**Correct program:** `main.rk`:

```rock
add: I64 -> I64 -> I64
add = left, right -> left + right

main = !->
    add 20, 22 .println!
```

From the directory containing `rock.toml`, run:

```console
$ rock run
42
```

The manifest lets the editor and build command agree on the program entry, path dependencies, and standard library. The `+` operator and `println` method in this example come from the standard library selected by the project tooling.

## Connect Neovim

The separate [rock.nvim](https://github.com/rock-lang-org/rock.nvim) plugin uses Neovim's native LSP APIs and requires Neovim 0.11 or newer; no `nvim-lspconfig` dependency is needed.

Clone the repository if you do not already have a checkout:

```console
$ git clone https://github.com/rock-lang-org/rock.nvim.git
```

Add the plugin repository root to `runtimepath` in `init.lua` before setup. Replace the path with your checkout's absolute path:

```lua
vim.opt.runtimepath:prepend("/absolute/path/to/rock.nvim")
require("rock").setup()
```

These snippets install or load Lua integration, not the compiler. The toolchain installation provides `rock-lsp`. By default, the plugin uses `rock-lsp` on `PATH`. You can also set the command explicitly:

```lua
require("rock").setup({
  cmd = { "rock-lsp" },
})
```

If Neovim's environment does not include the command, use an absolute path in `cmd` instead. Start Neovim after activating the toolchain shell environment so that both the server and its compiler dependencies can be found.

Open `main.rk`, then inspect the integration:

```vim
:set filetype?
:checkhealth rock
:RockLspInfo
```

The filetype should be `rock`. The health check verifies the Neovim APIs, executable, setup, and active clients; it is not a compilation check. `RockLspInfo` shows the command and client root, which is useful when diagnosing a server-path problem.

With the cursor on `left` in the body of `add` in the correct program, request hover information:

```vim
:lua vim.lsp.buf.hover()
```

The server can show the parameter's `I64` type from the function signature. With the cursor inside `add 20, 22`, request call information:

```vim
:lua vim.lsp.buf.signature_help()
```

Signature help identifies the function's parameters and the active argument. Commas trigger requests automatically in supporting clients; an explicit request is useful because Rock calls do not need parentheses. The plugin does not add custom key mappings, so these commands work without relying on a particular user's keybindings.

Tree-sitter highlighting is separate. [tree-sitter-rock](https://github.com/rock-lang-org/tree-sitter-rock) supplies the grammar and queries, but the LSP setup above does not install that parser. The [VS Code extension](https://github.com/rock-lang-org/vscode-rock) is also maintained separately.

## Reading a source diagnostic

**Intentional error: an integer annotation with a Boolean value.**

```rock
main = !->
    count: I64 = true
    count.println!
```

Replace `main.rk` with this example and save it, then run `rock build`. Compilation should fail: `true` has type `Bool`, but the binding requires `I64`. The following reading order applies even as exact diagnostic wording and terminal layout evolve:

1. Read the message and any bracketed family such as `type`. The family describes the kind of problem, not a unique numbered error.
2. Read the displayed file path and source position. A project can contain several files; do not assume the report refers to the file currently open in the editor.
3. Read the primary underline and its label. It marks the source expression or construct associated with the failure, not necessarily the only place you should edit.
4. Read secondary labels, if present. They can show another constraint, an earlier access, or a declaration in another file.
5. Read any note or help text before changing the program. Not every diagnostic includes these sections.

For this example, compare the annotation with the initializer. If `count` is meant to be a number, provide a number rather than changing an unrelated print call.

**Corrected program:**

```rock
main = !->
    count: I64 = 3
    count.println!
```

`rock run` prints `3`. If the intended value were a flag instead, a `Bool` annotation and a suitable binding name would express that different intent. A compiler error tells you which constraints disagree; it cannot choose the intended design for you.

In Neovim, put the cursor at the reported location and open the full diagnostic text:

```vim
:lua vim.diagnostic.open_float()
```

The LSP message includes available notes and help. Secondary labels are sent as related diagnostic information, though how they are displayed or navigated depends on the client. The terminal build report remains useful when an editor only shows a short message.

## Following an earlier borrow

Some errors concern the relationship between two operations. Here is a complete program with a shared reference that is still needed after an assignment.

**Intentional error: changing a value while it is borrowed.**

```rock
main = !->
    mut number: I64 = 1
    view = &number
    number = 2
    *view .println!
```

Run `rock build` to check this example. The assignment conflicts with the shared borrow: `view` is used afterward, so its access must remain valid across the assignment. A borrow-conflict report can label both the conflicting access and where the borrow was introduced. Read those labels together rather than deleting the highlighted assignment without considering the later use.

**Corrected program: finish using the reference before assigning.**

```rock
main = !->
    mut number: I64 = 1
    view = &number
    *view .println!
    number = 2
    number.println!
```

This version prints `1` and then `2`. The last use of `view` now precedes the assignment. See [References and Borrowing](../language/references.md) for the ownership rules behind this correction.

The editor may show no error for the incorrect version: its main-project analysis checks the source frontend, including types, but does not run borrow checking, code generation, or linking. A clean editor diagnostic list is not proof that `rock build` will succeed.

## When the location is not your source

Source-aware diagnostics keep labels associated with the file and source text they describe. A label in another module, a macro-related source location, or source retained with a dependency artifact is not an instruction to edit the same line number in `main.rk`. Follow the displayed path and read the related labels. Artifact-backed excerpts can describe the source used to build the artifact rather than your current working copy.

Not all failures have a meaningful source range. A missing file, invalid project configuration, unreadable artifact, or missing toolchain component can be reported with a file, project, artifact, or toolchain location instead. If no source text is available, a terminal report can fall back to a path and byte offsets; those offsets are not line numbers. In the editor, a project or toolchain failure may appear at the start of a buffer simply because it has no source range.

For setup failures, use this order:

1. Run `rock build` from the manifest directory and read the original diagnostic before the final command-failure summary.
2. Check the named path: does `rock.toml` select an existing entry, and do its path dependencies exist?
3. Check that the selected toolchain has its matching standard library; follow [Installation](installation.md) if packaging or selection is incomplete.
4. After correcting files or configuration, save and restart editor analysis if its report has not refreshed.

Do not change valid source to work around a missing toolchain. Likewise, an `internal` diagnostic is a compiler/tooling failure, not a request to guess at syntax changes; retain the full message and a small reproducing project when reporting it.

## Keep feedback current

The server finds the nearest ancestor `rock.toml` and analyzes its entry graph. Unsaved open source buffers overlay that graph, while unopened source comes from disk. Files outside the graph are not automatically analyzed just because they are under the project directory. Dependency packages are consumed as artifacts, so save their changes before refreshing a consuming project.

Dependency resolution can build artifacts and is cached. There is no automatic manifest/dependency file-watcher refresh; after saving changes to project configuration or selecting another toolchain, save a Rock buffer or restart:

```vim
:RockLspRestart
```

Diagnostics are currently published to open files rather than collected into a complete workspace index. Hover and signature help require a successful analysis; after an invalid edit, the server retains the previous successful snapshot, so information can be stale. Refreshing one buffer also does not guarantee that all other buffers' diagnostics have been cleared. Save and run `rock build` when editor feedback is incomplete or disagrees with your expectations.

The server currently provides diagnostics, hover, and signature help, not completion, go-to-definition, rename, code actions, or LSP formatting. Use `rock format` for the configured entry and inspect its changes, then `rock build` and `rock run` for the saved program. [Tooling Reference](../reference/tooling.md) lists server overrides and plugin options when you need more control.
