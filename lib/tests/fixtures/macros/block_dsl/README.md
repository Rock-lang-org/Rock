# External block-DSL fixture

This ordinary procedural package is a compiler acceptance fixture, not an HTML
implementation. Its source owns every interpretation rule; the compiler only
captures the indented UTF-8 input, supplies the generic SDK, and checks output.

The tiny fixture language reads one or two decimal digits immediately after the
first `=`. The remaining payload is opaque, except that `@` is reserved and
produces a library diagnostic at that exact input byte. Tags, apostrophes,
unmatched quotes/brackets, comment-looking text, backticks, Unicode, odd margins
and tabs have no Rock interpretation inside the block.

- `render` emits the decoded number as a Rock expression.
- `declare` emits an ordinary `generated` function returning that number.
- `raw` deliberately forwards the original DSL as output; ordinary generated
  Rock parsing must reject it.
- `expression.rk`, `items.rk` and `forward.rk` are complete no-std consumers used
  by both native SDK and real CLI acceptance. They return process status 42 and
  retain a following dedented `after` declaration. `forward.rk` forwards a sealed
  expression containing the nested block through a declarative macro.
- `error.rk` places the reserved byte after multibyte text to prove exact source
  ranges rather than character-index guesses.

Contributor commands, from the repository root:

```sh
cargo test -p rock macro_b_20 -- --nocapture
cargo test -p rock-lib --test macro_sdk -- --nocapture
```

Run these suites serially. They rebuild explicit SDK/std dependencies and need
the normal LLVM 18 development environment. No runtime SDK/std dependency is
supplied to the consumer, and no compiler-owned HTML or DSL policy is added.
