# Enums and Pattern Matching

An enum is a value that can have one of several variants. Each variant may carry a different payload, but every variant belongs to the same enclosing type.

## Declaring and constructing variants

```rock
enum Message
    Quit
    Move I64, I64
    Write &Str

main = !->
    stop = Message::Quit
    movement = Message::Move 4, -2
    text = Message::Write "hello"
    stop_text = match stop
        Message::Quit => "quit"
        Message::Move x, y => "move"
        Message::Write value => value
    stop_text.println!
    movement_text = match movement
        Message::Quit => "quit"
        Message::Move x, y => "move"
        Message::Write value => value
    movement_text.println!
    text_text = match text
        Message::Quit => "quit"
        Message::Move x, y => "move"
        Message::Write value => value
    text_text.println!
```

`Message::Quit` has no payload, `Message::Move` has two `I64` payloads, and `Message::Write` has one `&Str` payload. Construction qualifies the variant with the enum path so the compiler knows which type owns it.

Generic enums use the same declaration form:

```rock
enum Choice Left, Right
    First Left
    Second Right

main = !->
    choice: Choice &Str, &Str = Choice::First "left"
    label = match choice
        Choice::First value => value
        Choice::Second value => "right"
    label.println!
```

The prelude's `Option T` and `Result T, E` are ordinary generic enums of this kind. `Option` has `Some T` and `None`; `Result` has `Ok T` and `Err E`.

## Matching variants

A constructor pattern checks the variant and binds each payload:

```rock
enum Message
    Quit
    Move I64, I64
    Write &Str

describe: Message -> &Str
describe = message ->
    match message
        Message::Quit => "quit"
        Message::Move x, y => "move"
        Message::Write text => text

main = !->
    describe Message::Move 4, -2 .println!
    describe Message::Write "hello" .println!
```

For a `Move`, `x` and `y` are introduced only in that arm and have type `I64`. For a `Write`, `text` has type `&Str`. Every arm returns `&Str`, which satisfies the function signature.

Use `_` when a value is deliberately ignored:

```rock
enum Message
    Quit
    Move I64, I64
    Write &Str

is_quit: Message -> Bool
is_quit = message ->
    match message
        Message::Quit => true
        _ => false

main = !->
    is_quit Message::Quit .println!
    is_quit Message::Write "hello" .println!
```

Qualified patterns such as `Message::Quit` are clearer than unqualified variant names and avoid collisions when several enums have similarly named variants.

## Guards

An `if` guard runs after the structural pattern succeeds:

```rock
classify: I64 -> &Str
classify = value ->
    match value
        number if number < 0 => "negative"
        0 => "zero"
        _ => "positive"

main = !->
    classify 0 - 3 .println!
    classify 0 .println!
    classify 8 .println!
```

The first arm binds `number`, then checks its guard. If the guard is false, matching continues with the next arm. Guards may use names introduced by their pattern.

> **Current status:** A guard cannot currently consume a non-copy enum payload binding, even when matching a borrowed scrutinee. Match without the guard and put the condition inside the arm instead.

## Payload bindings and wildcards

An enum payload can contain several values. A constructor pattern binds those values positionally, while `_` ignores a payload that the arm does not need:

```rock
enum Packet
    Data I64, &Str
    Empty

describe: Packet -> &Str
describe = packet ->
    match packet
        Packet::Data number, text if number > 0 => text
        Packet::Data _, _ => "nonpositive data"
        Packet::Empty => "empty"

main = !->
    describe Packet::Data 3, "three" .println!
    describe Packet::Data 0 - 1, "negative" .println!
    describe Packet::Empty .println!
```

The first arm binds both payloads and checks the guard. If the guard is false, the second arm matches any `Data` payload without introducing names. Tuple destructuring assignments are covered in the bindings chapter; the current compiler does not yet execute refutable tuple matches.

## Matching through a reference

Matching an owned enum by value can move a non-copy payload into an arm binding. Matching through a shared reference borrows the payload instead:

```rock
enum Message
    Quit
    Write &Str

show_message: &Message -> ()
show_message = message ->
    match *message
        Message::Write text => text.println!
        Message::Quit => "quit".println!
    return

main = !->
    message = Message::Write "borrowed payload"
    show_message &message
```

The `*message` scrutinee is a borrowed match. The original `message` remains available after `show_message` returns because the match did not consume the enum.

## Exhaustiveness and arm order

List every variant or finish with a wildcard:

```rock
enum Color
    Red
    Green
    Blue

name: Color -> &Str
name = color ->
    match color
        Color::Red => "red"
        Color::Green => "green"
        Color::Blue => "blue"

main = !->
    name Color::Blue .println!
```

The current compiler does not diagnose every incomplete enum match, so exhaustive source is an important programmer responsibility. Put specific patterns before broad ones; a wildcard first would make later arms unreachable in intent even where the compiler does not report it.

## Pattern reference

The forms used in this chapter are:

| Form | Meaning |
| --- | --- |
| `_` | Ignore a value |
| `name` | Bind a value |
| `mut name` | Bind it for a mutable operation |
| `42`, `true`, `"x"` | Match a literal |
| `Type::Variant value` | Match an enum variant and bind its payload |
| `&pattern`, `&mut pattern` | Match through a reference pattern |

## Common mistakes

- Binding a payload name in one arm and trying to use it in another arm.
- Placing `_` before a specific variant pattern.
- Treating a guard as a replacement for the structural pattern; the pattern still has to match first.
- Consuming an owned enum when a shared-reference match would preserve it.
- Adding a variant without updating every exhaustive match.
