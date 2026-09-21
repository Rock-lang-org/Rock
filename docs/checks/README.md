# Book Highlighting

## Compile Examples

Compile every Rock fence in the README and book, plus the sources under
`docs/examples`, using the current compiler:

```sh
cargo build --release -p rockc
node docs/checks/compile-examples.cjs
```

The runner rebuilds the stdlib, assembles the companion files in the modules and
packages chapters, and builds their dependencies. The HTTP examples require the
`rock_http` checkout and revision documented in the HTTP chapter, at
`../rock_http` by default. Set `ROCK_HTTP_ROOT` to use another checkout location,
or `ROCKC` to select another compiler binary. Missing dependencies fail the check;
they are not skipped.

Intentional rejection examples have a `compile-fail` HTML comment immediately
before their fence naming the required diagnostic. They pass only when the
compiler rejects them with that diagnostic, not when it crashes or reports an
unrelated error. Logs are retained in the temporary directory printed by the
runner. This checks compilation to object files, not runtime behavior or linking.

## Highlighting

The book requires Node.js 22, mdBook 0.5.4, tree-sitter-cli (CI pins
0.26.9), and a C compiler for the Rock parser. The extension's pinned CLI 0.27.0
is also supported. No npm dependencies are needed for the book.

CI pins mdBook 0.5.4 so preprocessors run from the book directory, consistently
with local builds. mdBook 0.4 runs them from the caller's working directory and
cannot use the configured relative command when invoked as `mdbook build docs`.

From the repository root:

```sh
cargo install tree-sitter-cli --version 0.26.9 --locked
node docs/checks/fetch-grammar.cjs
node --test docs/checks/rock-highlight.test.cjs
mdbook build docs
node docs/checks/verify-book.cjs
```

To preview the book locally after fetching the grammar:

```sh
cd docs
mdbook serve --port 3001
```

If the preprocessor reports `Missing Rock AST highlight query`, run
`node checks/fetch-grammar.cjs` from `docs`, then retry the build or serve command.

`rock-highlight.cjs` implements the mdBook preprocessor protocol. It batches Rock
fences through the tree-sitter CLI using `queries/highlights.scm`
and `queries/locals.scm` from the standalone
[tree-sitter-rock](https://github.com/rock-lang-org/tree-sitter-rock) repository, against
the real Rock syntax tree. The generated parser C source is ignored, so generation is
required on a fresh checkout. The fetch script checks out the exact commit in
`grammar-revision.txt` under ignored `docs/.deps/tree-sitter-rock/` and generates
the parser. Change that pin explicitly to adopt grammar updates; no submodule or
sibling checkout is needed. Grammar and query errors fail the build.

The preprocessor retains tree-sitter capture classes such as `function`,
`operator assignment`, and `variable builtin`. Style these under
`code.language-rock` in `docs/theme/rock.css`. Rock token classification must stay
in the AST query, not a JavaScript tokenizer. The `nohighlight` class prevents
mdBook's Highlight.js pass from replacing the spans; the ordinary `pre > code`
structure retains mdBook's copy buttons. Source round-trip checks ensure copied
text has no table line numbers, injected markup, or changed whitespace.

The locals query propagates parameter colors through their lexical scopes,
including captured references and reassignment. Member names and unrelated
body bindings retain their own roles rather than inheriting colors by spelling.

Enum variants use `constant variant`, separately from their owning types. The
query marks declarations, qualified constructors, and patterns directly. A
per-fence pass resolves AST-captured short value names against that fence's
variant declarations and explicit imports, including forward references. Type
annotations are not candidates, and declarations never leak between examples.

The preprocessor resolves grammar and query paths relative to its script, not
the caller's working directory. mdBook runs the configured command from `docs`.
The verifier checks every chapter's rendered Rock blocks against its Markdown
source and checks individual AST captures in the Resource/Drop example.
