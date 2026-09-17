# Rock Language Syntax Specification

## Overview

Rock is a functional programming language with Haskell-inspired syntax, featuring strong typing, pattern matching, traits, and modern language constructs. This specification documents the complete syntax as implemented in the compiler.

## 1. Lexical Structure

### 1.1 Keywords
```
struct, enum, trait, impl, if, then, else, for, in, while, loop, macro,
true, false, return, continue, break, infix, mod, extern, match, unsafe,
type, mut
```

### 1.2 Operators
- **Arithmetic**: `+`, `-`, `*`, `/`
- **Comparison**: `=`, `!`, `<`, `>`
- **Special**: `$`, `|`, `&`
- **Custom operators**: User-defined with `infix` declaration

### 1.3 Delimiters and Punctuation
- **Parentheses**: `(`, `)`
- **Brackets**: `[`, `]`
- **Arrows**: `->` (function), `=>` (match/macro)
- **Punctuation**: `,` (comma), `:` (colon), `::` (double colon), `.` (dot), `..` (double dot), ` .` (spaced dot)
- **Special**: `@` (self reference), `?` (error propagation), `_` (wildcard), `=` (assignment)

### 1.4 Literals
- **Numbers**: `123`, `456`
- **Floats**: `3.14`, `2.718`
- **Strings**: `"hello world"`
- **Characters**: `'a'`, `'x'`
- **Booleans**: `true`, `false`
- **Arrays**: `[1, 2, 3]`, `[]`

### 1.5 Identifiers
- **Variables**: lowercase starting: `variable`, `my_var`
- **Types**: uppercase starting: `MyType`, `String`
- **Self references**: `@field`, `@method`

### 1.6 Comments
- **Line comments**: `// comment text`
- **Block comments**: `/* comment text */` (planned)

### 1.7 Indentation
Rock uses significant whitespace for block structure. Indentation levels are tracked and must be consistent within a block.

## 2. Types

### 2.1 Basic Types
```rock
// Primitive types
Int8, Int16, Int32, Int64
UInt8, UInt16, UInt32, UInt64
Float32, Float64
Bool
String
Char
```

### 2.2 Composite Types

#### Function Types
```haskell
// Function type syntax
add : Int -> Int -> Int
callback : (String -> Bool) -> String
```

#### Array Types
```haskell
numbers : [Int]
matrix : [[Float]]
```

#### Tuple Types
```haskell
point : (Int, Int)
triple : (String, Int, Bool)
```

#### Reference Types
```haskell
// Mutable reference
mut_ref : &mut Int
// Immutable reference  
ref : &Int
```

#### Pointer Types
```haskell
// Raw pointer (unsafe)
ptr : *Int
```

#### Unit Type
```haskell
// Unit type (empty tuple)
unit : ()
```

### 2.3 Generic Types
```haskell
// Generic struct
struct Container T
    value: T

// Generic enum
enum Result T, E
    Ok T
    Err E
```

### 2.4 Kinds and Type Constructors

Ordinary types have kind `Type`. A generic type constructor has a function kind;
for example, `Option` has kind `Type -> Type` and `Result` has kind
`Type -> Type -> Type`.

```haskell
// F accepts one type argument.
trait Functor for F _
    fmap: M -> F A -> F B where M: FnMut A, B

// Parenthesized annotations declare higher-order kinds explicitly.
(H: (Type -> Type) -> Type)

// A section fixes Result's error type and leaves its value type open.
Result _, E

// Type lambdas are the canonical explicit form of the same constructor.
\T -> Result T, E
```

The `_` placeholder is valid only in a constructor section. Constructor
application associates to the left, while a type lambda extends as far right as
its body. Unsaturated constructors are compile-time terms and cannot be used as
runtime value types.

## 3. Expressions

### 3.1 Primary Expressions

#### Literals
```haskell
42          // number
3.14        // float
"hello"     // string
'c'         // character
true        // boolean
[1, 2, 3]   // array
```

#### Identifiers
```haskell
variable    // variable reference
MyType      // type reference
@field      // self field reference
```

#### Parenthesized Expressions
```haskell
(1 + 2)     // grouped expression
```

### 3.2 Function Calls
```haskell
// Function call
foo arg1, arg2

// Method call with dot notation
obj.method arg

// Chained calls
obj.method1!.method2 arg
```

### 3.3 Array/Tuple Access
```haskell
// Array indexing
arr[0]
arr[i + 1]

// Tuple field access
tuple.0
tuple.1
```

### 3.4 Binary Operations
```haskell
// Arithmetic
a + b
a - b
a * b
a / b

// Comparison
a == b
a != b
a < b
a > b

// Custom operators
a |> b      // pipe operator
a >>= f     // method-backed bind operator
```

Custom infix operators currently have two precise resolution paths:
- Function-backed operators resolve through normal function name resolution, for example `|>`.
- Method-backed operators resolve against the left operand type, for example stdlib operators like `>>=`, `<|>`, and `<&>`.

### 3.5 Unary Operations
```haskell
// Prefix operators
-x          // negation
!x          // logical not
*ptr        // pointer dereference
```

## 4. Statements

### 4.1 Expression Statements
```haskell
// Any expression can be a statement
foo!
1 + 2
obj.method!
```

### 4.2 Assignment
```haskell
// Variable assignment
x = 42
name = "Alice"

// Pattern assignment (destructuring)
(a, b) = (1, 2)
[first, ..rest] = array

// Type annotation
value: Int = 42
```

### 4.3 Control Flow Statements
```haskell
// Return
return
return value

// Break/Continue
break
continue
break value
continue value
```

## 5. Control Flow

### 5.1 Conditional Expressions
```haskell
// Basic if-then-else
if condition then value1 else value2

// Multi-line if
if condition
then
    statement1
    statement2
else
    statement3

// If-let pattern matching
if Ok value = result
then process value
else handle_error!
```

### 5.2 Loops

#### While Loops
```haskell
while condition
    body_statement
```

#### For Loops
```haskell
for item in collection
    process item
```

#### Infinite Loops
```haskell
loop
    statement
    if condition then break
```

### 5.3 Pattern Matching
```haskell
match expression
    pattern1 => result1
    pattern2 if guard => result2
    _ => default_result
```

#### Pattern Types
```haskell
// Literal patterns
match x
    42 => "forty-two"
    "hello" => "greeting"

// Variable binding
match x
    value => process value

// Tuple patterns
match point
    (0, 0) => "origin"
    (x, 0) => "on x-axis"
    (0, y) => "on y-axis"
    (x, y) => "general point"

// Array patterns
match list
    [] => "empty"
    [x] => "single element"
    [first, ..rest] => "multiple elements"

// Constructor patterns
match result
    Ok value => value
    Err error => handle error

// Guard patterns
match x
    n if n > 0 => "positive"
    n if n < 0 => "negative"
    _ => "zero"

// Binding patterns
match data
    value @ (x, y) if x > y => use value
```

## 6. Functions

### 6.1 Function Declarations
```haskell
// Basic function
add = x, y -> x + y

// Function with type signature
add : Int -> Int -> Int
add = x, y -> x + y

// Multi-line function body
process = data ->
    step1 = transform data
    step2 = validate step1
    finalize step2
```

#### Unit-Returning Functions and Entry Points

The discard arrow `!->` evaluates a function or lambda body for effects, discards its result, and returns unit (`()`). Prefer it for an ordinary program's entry point:

```haskell
main = !->
    "Hello, Rock!".println!
```

When `main` returns unit, normal completion maps to process exit status `0`; no final `0` expression is needed. Discarding a value does not inspect it for errors or produce a failure exit status. Use the ordinary `->` arrow when intentionally returning an integer exit status:

```haskell
main = ->
    "This program intentionally reports failure.".println!
    1
```

### 6.2 Lambda Expressions
```haskell
// Lambda syntax
lambda = x -> x + 1

// Multi-parameter lambda
combine = x, y -> x + y

// Function shorthand
plus_one = (+1)
multiply_by_two = (*2)
```

### 6.3 Function Features

#### Currying
```haskell
add = x, y -> x + y
add_five = add 5
result = add_five 3  // result is 8
```

#### Higher-Order Functions
```haskell
apply = f, x -> f x
map = f, list -> // implementation
```

## 7. Data Types

### 7.1 Struct Declarations
```haskell
// Basic struct
struct Point
    x: Int
    y: Int

// Struct with default values
struct Config
    debug: Bool = false
    timeout: Int = 30

// Generic struct
struct Container T
    value: T
    count: Int

// Public fields
struct PublicData
    < public_field: String
    private_field: Int
```

### 7.2 Struct Instantiation
```haskell
// Named field syntax
point = Point
    x: 10
    y: 20

// Constructor pattern
point = Point::new 10, 20
```

### 7.3 Enum Declarations
```haskell
// Simple enum
enum Color
    Red
    Green  
    Blue

// Enum with data
enum Result T, E
    Ok T
    Err E

// Enum with named fields
enum Message
    Quit
    Move
        x: Int
        y: Int
    Write String
    ChangeColor Int, Int, Int
```

## 8. Traits and Implementations

### 8.1 Trait Declarations
```haskell
// Basic trait
trait Display
    @show : String

// Trait with default implementation
trait Printable
    @print = -> @show!.output!
    @show : String

// Generic trait
trait Container T
    @get : T
    @set : T -> ()
```

### 8.2 Implementations
```haskell
// Implement trait for type
impl Display for Point
    @show = -> "Point(" + @x.show! + ", " + @y.show! + ")"

// Standalone implementation
impl Point
    new = x, y -> Point x: x, y: y
    @distance_from_origin = -> sqrt (@x * @x + @y * @y)

// Generic implementation
impl Container T for Box T
    @get = -> @value
    @set = value -> @value = value
```

### 8.3 Constructor Traits and Functional Abstractions

Traits may target type constructors. Constructor trait members are static and
are called through the implementing constructor or an explicitly qualified
partial constructor.

```haskell
mapped = Option::Functor::fmap inc, (Option::Some 1)
mapped_result = (Result _, String)::Functor::fmap inc, result
```

The standard library defines `Bifunctor`, `Functor`, `Applicative`, `Monad`,
`Foldable`, and `Traversable` as ordinary source traits. Their laws use
extensional equality: Bifunctor and Functor preserve identity and composition;
Applicative preserves identity, composition, homomorphism, and interchange;
Monad preserves both identities and associativity; Traversable preserves
identity, composition, and naturality.

`Bifunctor` targets binary constructors. For `Result T, E`, `first` maps the
success parameter `T`, `second` and `<!>` map the error parameter `E`, and
`bimap` maps both. The generic functional operators are `<$>`, `<&>`, `<*>`,
`<!>`, and `>>=`; their meanings live in stdlib source rather than the compiler.

These operations are strict and evaluate finite containers left-to-right.
`Functor::fmap` and `Foldable::foldl` consume their containers. `Vec::map_ref`
and `Vec::for_each` are explicit borrowed alternatives, while consuming Vec
operations move each element without implicit cloning and remain stack-safe.
`Option` and `Result _, E` implement all five unary traits, while bare `Result`
implements the binary `Bifunctor` trait. `Vec` implements `Functor`, `Foldable`,
and `Traversable`, but not `Applicative` or `Monad`: Cartesian application
requires cloning and zipped application does not have list monad semantics.

## 9. Modules and Imports

### 9.1 Module Declarations
```haskell
// Inline module
mod utils
    helper = x -> x + 1
    
// External module (references utils.rk file)
mod utils
```

### 9.2 Import/Export Syntax
```haskell
// Import specific items
> std::collections::HashMap
> utils::helper

// Export items
< MyStruct
< my_function
```

## 10. Macros

### 10.1 Macro Declarations
```haskell
macro debug_print
    $msg:expr =>
        if DEBUG
        then print $msg

// Macro with repetition
macro vec
    $($item:expr),* =>
        [$(item),*]
```

### 10.2 Macro Invocations
```haskell
// Macro call
%debug_print "Hello, world!"

// Macro with arguments
%vec 1, 2, 3, 4
```

## 11. Operators

### 11.1 Custom Operator Definitions
```haskell
// Infix operator with precedence
infix 5 |>
|> = x, f -> f x

// Usage
result = value |> transform |> validate
```

### 11.2 Operator Precedence
Higher declared precedence binds more tightly. Function application has precedence `8`: operators above `8` become part of the final function argument, while operators at or below `8` apply to the call result. For example, `double x + 1` means `double (x + 1)` when `+` has precedence `9`, while `Option::Some x <&> transform` applies `<&>` to `Option::Some x` when `<&>` has precedence `8`.

A tight dot binds directly to its receiver: `value.method!`. A spaced dot has lower precedence than the complete expression on its left, so `double x + 1 .println!` prints the result of `double (x + 1)`. Parentheses remain available when explicit grouping is needed, such as `(double x) + 1`.

## 12. Unsafe Code

### 12.1 Unsafe Blocks
```haskell
main = !->
    unsafe
        ptr: *Int = get_raw_pointer!
        value = *ptr  // dereference raw pointer
        process value
```

## 13. Error Handling

### 13.1 Error Propagation
```haskell
// Question mark operator for error propagation
result = risky_operation?

// Chained error propagation
final_result = step1? |> step2? |> step3?
```

### 13.2 Result Types
```haskell
// Function returning Result
divide : Int -> Int -> Result Int, String
divide = a, b ->
    if b == 0
    then Err "Division by zero"
    else Ok (a / b)
```

This specification covers the core syntax of the Rock programming language as implemented in the compiler. The language combines functional programming concepts with modern syntax and safety features.
