# Learn Rock in Y Minutes

Rock is a natively compiled (LLVM) language with a functional style and
explicit ownership. It borrows layout and lambdas from LiveScript, type
classes and operators from Haskell, and ownership from Rust. Types are
inferred almost everywhere.

Every code block below is a complete `src/main.rk`: create a project with
`rock new tour && cd tour`, paste a block, and `rock run`. The std prelude
(`Option`, `Result`, `Vec`, `String`, operators, `%println`, ...) is in scope
automatically.

## 1. Hello, comments and layout

```haskell
// Line comment.
/* Block comment. */

// Indentation delimits blocks; there are no semicolons or braces.
// `!->` defines a function whose result is discarded: it returns `()`,
// which means exit status 0 for `main`.
main = !->
    "Hello, Rock!".println!     // `.method!` calls a method with no arguments
```

## 2. Values and bindings

```haskell
main = !->
    // Bindings are immutable by default; types are inferred.
    answer = 42
    ratio = 3.5
    ok = true
    letter = 'R'                // Char: one Unicode scalar
    name = "Rock"               // string literal: &Str (borrowed UTF-8)

    // Optional annotations.
    big: I64 = 1000
    small: U8 = 7

    // `mut` allows reassignment and mutable borrows.
    mut counter = 0
    counter = counter + 1

    // %println shows every argument and joins them.
    %println answer, " ", ratio, " ", ok, " ", letter, " ", name
    %println big, " ", small, " ", counter

    // %format builds a String instead of printing.
    message = %format "counter = ", counter
    message.println!
```

## 3. Arrays, tuples, slices, ranges

```haskell
main = !->
    values = [10, 20, 30]               // fixed-length array [I64; 3]
    sevens = [7; 4]                     // [7, 7, 7, 7]
    pair = ("total", 3)                 // tuple

    pair.0.println!                     // tuple fields: .0, .1, ...
    values[1].println!                  // indexing
    values[..2].println!                // borrowed slice: [10, 20]
    sevens.println!

    mut sum = 0
    for i in 0..3                       // 0, 1, 2  (exclusive)
        sum = sum + values[i]
    for i in 1..=2                      // 1, 2     (inclusive)
        sum = sum + i
    sum.println!                        // 63
```

## 4. Control flow — everything is an expression

```haskell
category = temperature ->
    if temperature < 0
        "freezing"
    else if temperature < 20
        "cool"
    else
        "warm"

main = !->
    category 12 .println!

    // One-line if uses `then`.
    parity = if 4 % 2 == 0 then "even" else "odd"
    parity.println!

    mut n = 0
    while n < 5
        n = n + 1
        if n == 2
            continue
        if n == 4
            break
        n.println!                      // 1, 3

    loop
        "looping once".println!
        break
```

## 5. Functions

```haskell
// Signatures are optional; inference handles both of these.
add: I64 -> I64 -> I64
add = left, right -> left + right

square = x -> x * x

// Multi-line body: the last expression is the result.
hypot_sq = a, b ->
    aa = square a
    bb = square b
    aa + bb

// Zero-argument function, called with `!`.
answer = -> 42

// Early return.
clamp = value ->
    if value > 100
        return 100
    value

main = !->
    // Arguments are comma-separated; no parentheses needed.
    add 2, 3 .println!                  // a spaced ` .` applies to the whole call
    hypot_sq 3, 4 .println!
    answer!.println!
    clamp 500 .println!

    // A nested call takes its own arguments.
    square add 1, 2 .println!           // square (add 1, 2) = 9
    (square 3) + 1 .println!            // parentheses only group
```

## 6. Lambdas, currying, holes, sections, pipelines

```haskell
add = left, right ~> left + right       // `~>` makes it curried
multiply = left, right -> left * right  // `->` is not curried

main = !->
    add10 = add 10                      // partial application
    add10 5 .println!                   // 15

    double = multiply _, 2              // `_` is a call hole
    double 21 .println!                 // 42

    // `|>` pipes a value into the next function.
    5
        |> add 10
        |> multiply _, 2
        |> (.println!)                  // method section: x -> x.println!

    5
        |> (* 2)                        // operator section: x -> x * 2
        |> value -> value + 1           // inline lambda
        |> (.println!)

    // Multi-parameter lambdas are grouped when passed inline.
    apply = f, a, b -> f a, b
    apply (l, r -> l - r), 10, 3 .println!
```

## 7. Structs, impls and receivers

```haskell
struct Point
    < x: I64                            // `<` exports (makes public)
    < y: I64

impl Point
    // Associated function: Point::new
    new = x, y ->
        Point
            x: x
            y: y

    @sum = -> @x + @y                   // `@`  : shared borrow of self (`@x` = self.x)

    ^@shift = dx !->                    // `^@` : mutable borrow of self
        self.x = self.x + dx

    ~@into_tuple = -> (self.x, self.y)  // `~@` : consumes self

main = !->
    mut p = Point::new 1, 2
    p.shift 10
    p.sum!.println!                     // 13
    t = p.into_tuple!                   // `p` is moved; can't be used now
    t.0.println!
```

## 8. Enums and pattern matching

```haskell
enum Shape
    Circle I64
    Rect I64, I64
    Empty

area = shape ->
    match shape
        Shape::Circle r          => 3 * r * r
        Shape::Rect w, h if w == h => w * w      // guard
        Shape::Rect w, h         => w * h
        Shape::Empty             => 0

// Generic enum.
enum Answer T
    Value T
    Missing

value_or = fallback, answer ~>
    match answer
        Answer::Value v => v
        Answer::Missing => fallback

main = !->
    area Shape::Circle 2 .println!
    area Shape::Rect 3, 4 .println!

    // Literal, tuple and wildcard patterns.
    match (1, "one")
        (0, _)    => "zero".println!
        (n, name) => %println n, " is ", name

    Answer::Value 42 |> value_or 0 |> (.println!)
    Answer::Missing |> value_or 7 |> (.println!)
```

## 9. Option, Result and `?`

```haskell
divide = numerator, denominator ->
    if denominator == 0
    then Result::Err "division by zero"
    else Result::Ok numerator / denominator

// `?` unwraps Ok/Some or returns the Err/None early.
quarter = value -> divide (divide value, 2?), 2

main = !->
    quarter 84 .println!                // Ok(21)
    divide 1, 0 .println!               // Err(division by zero)

    maybe = Option::Some 5
    match maybe
        Option::Some v => v.println!
        Option::None   => "nothing".println!

    nothing = Option::None
    nothing.unwrap_or 9 .println!       // 9
```

## 10. Ownership, borrowing, cloning

```haskell
length = text -> text.len!              // takes a borrow if given one

consume = text !-> text.println!        // takes ownership

main = !->
    text = String::from "Rock"          // owned, heap-allocated
    length &text .println!              // `&` lends a shared borrow
    copy = text.clone!                  // explicit deep copy
    consume text                        // ownership moves here
    // text.println!                    // error: use after move
    copy.println!

    mut n = 1
    bump = r !-> *r = *r + 1            // `*` dereferences
    bump &mut n                         // mutable borrow
    n.println!                          // 2
```

Values are dropped deterministically at end of scope. Implement `Drop` to run
code on drop. Tuples and arrays copy structurally; structs and enums copy only
with an explicit `impl Copy for Type` (all fields `Copy`, no `Drop`).

```haskell
struct Resource
    < id: I64

impl Drop for Resource
    ~@drop = !-> %println "dropping ", self.id

struct Meters
    < value: I64

impl Copy for Meters

main = !->
    r = Resource
        id: 7
    %println "using ", r.id
    a = Meters
        value: 3
    b = a                               // copied: `a` is still usable
    %println a.value + b.value
    // prints "dropping 7" when `r` goes out of scope
```

## 11. Strings (UTF-8)

```haskell
main = !->
    text = String::from "aé中🦀"
    text.len!.println!                  // 10 — lengths are in bytes
    text[1..3].println!                 // é  — slices check char boundaries
    text.char_at 3 .println!            // Some(🦀) — index by Unicode scalar

    small = "aé🦀"
    for (offset, ch) in small.char_indices!
        %println offset, ": ", ch

    joined = String::from "Hello, " + "world"
    joined.println!
```

## 12. Collections

```haskell
main = !->
    numbers: Vec _ = [1, 2, 3, 4].into!     // array -> Vec
    mut more = Vec::new!
    more.push 5
    more.push 6
    more.len!.println!

    numbers.filter (v -> v % 2 == 0)        // filter borrows each element
        <&> (* 10)                          // map
        |> (.println!)                      // [20, 40]

    for item in &more
        item.println!

    mut scores = HashMap::new!
    scores.insert "ann", 10
    scores.insert "bob", 20
    match scores.get &"ann"
        Option::Some s => s.println!
        Option::None   => "missing".println!
```

## 13. Traits

```haskell
trait Area
    @area: I64

    // Default method.
    @describe: String
    @describe = -> %format "area = ", self.area!

struct Rect
    < w: I64
    < h: I64

impl Area for Rect
    @area = -> self.w * self.h

// Associated types.
trait Source
    type Item
    @read: Self::Item

impl Source for Rect
    type Item = I64
    @read = -> self.w

// Generic function with a bound.
print_area: &T -> () where T: Area
print_area = shape !-> shape.area!.println!

main = !->
    r = Rect
        w: 6
        h: 7
    print_area &r
    r.describe!.println!
    r.read!.println!
```

## 14. Generics and const generics

```haskell
struct Pair A, B
    < first: A
    < second: B

swap = pair -> Pair
    first: pair.second
    second: pair.first

// `N: usize` is a compile-time array length.
struct Buffer T, N: usize
    < data: [T; N]

impl Buffer T, N
    @len = -> N

main = !->
    p = swap Pair
        first: 1
        second: "one"
    p.first.println!

    buf: Buffer I64, 3 = Buffer
        data: [10, 20, 30]
    buf.len!.println!
```

## 15. Functional operators (Functor, Applicative, Monad...)

They are all ordinary std functions backed by traits.

```haskell
add = l, r ~> l + r
double = v -> v * 2
half_even = v -> if v % 2 == 0 then Option::Some v / 2 else Option::None

main = !->
    double <$> Option::Some 21 |> (.println!)        // fmap:  Some(42)
    Option::Some 21 <&> double |> (.println!)        // flipped fmap
    Option::None <|> Option::Some 7 |> (.println!)   // fallback: Some(7)

    Option::Some 84                                  // bind (Monad)
        >>- half_even
        >>- half_even
        |> (.println!)                               // Some(21)

    add <$> Option::Some 20 <*> Option::Some 22 |> (.println!)  // Applicative

    bad: Result I64, &Str = Result::Err "boom"
    bad <!> (e -> %format "error: ", e) |> (.println!)  // map the error side

    // Folding and traversing.
    values: Vec _ = [1, 2, 3].into!
    foldl (acc_x -> acc_x.0 + acc_x.1), 0, values .println!    // 6
```

## 16. `do` notation

Flat sequencing for anything with `bind` (`Option`, `Result`, your types).

```haskell
parse_digit = c ->
    if c >= '0' && c <= '9'
    then Option::Some 7
    else Option::None

sum_two = a, b -> do
    x <- parse_digit a                  // `<-` binds the success payload
    y <- parse_digit b
    total = x + y                       // plain binding
    pure total                          // wrap in the inferred carrier

main = !->
    sum_two '1', '2' .println!          // Some(14)
    sum_two '1', 'x' .println!          // None
```

## 17. Higher-kinded types: implement Functor yourself

```haskell
enum Wrap T
    Value T

impl Functor for Wrap                   // for the constructor, not `Wrap I64`
    fmap = mut mapper, wrapped ->
        match wrapped
            Wrap::Value v => Wrap::Value mapper.call_mut v

impl Wrap T
    ~@unwrap = ->
        match self
            Wrap::Value v => v

main = !->
    Wrap::Value 41 <&> (+ 1) |> (.unwrap!) |> (.println!)   // 42
```

## 18. Custom operators

```haskell
infix 9 %%                              // declare precedence (application is 8)

%% = left, right -> left * 10 + right   // define it like a function

main = !->
    4 %% 2 .println!                    // 42
```

Even `+` and `==` are declared this way in std; the compiler hard-codes none.

## 19. Macros

```haskell
macro make_constant
    $name:ident $value:expr =>
        $name = -> $value

%make_constant answer 6 * 7             // `%` invokes a macro

main = !->
    answer!.println!
```

Fragments: `ident`, `expr`, `ty`. Inspect expansion with `rock expand`.

## 20. Modules

`src/math.rk`:

```haskell
square = value -> value * value

< square                                // `<` exports
```

`src/main.rk`:

```haskell
mod math                                // loads math.rk next to this file
> math::square                          // `>` imports

main = !->
    square 6 .println!
```

## 21. Threads, files, FFI, unsafe

```haskell
> std::concurrency::thread::spawn
> std::io::fs::File
> std::io::IoError

extern abs: I32 -> I32                  // C function from the linker

write_file: &Str -> Result I64, IoError
write_file = path -> do
    file <- File::create path
    file.write_str "Hello from Rock\n"

unsafe set_i64: &mut I64 -> ()
set_i64 = reference !->
    pointer: *I64 = reference as *I64   // raw pointer
    unsafe *pointer = 7

main = !->
    spawn (-> 42)
        >>- (.join!)
        <&> (.println!)
        <!> _ !-> "thread failed".println!

    write_file "/tmp/rock-hello.txt" .println!

    abs -7 .println!

    mut v = 0
    unsafe set_i64 &mut v
    v.println!
```

## Tooling cheatsheet

```sh
rock new app        # new project (rock.toml + src/main.rk)
rock run            # build + run (debug)
rock run --release  # optimized
rock build -D unused  # turn unused warnings into errors
rock expand         # show macro-expanded source
rock clean          # remove build/
```

## Further reading

- The Rock Book: <https://rock-lang-org.github.io/Rock/>
- [README tour](../README.md) and [examples](../examples/)
