# Functions as Values

Functions and lambdas are values. They can be bound to names, passed as arguments, returned from other functions, and stored in generic containers. Their function type lists parameter types followed by the return type.

## Lambdas

A lambda has parameters, an arrow, and a body. The following function has the explicit signature `I64 -> I64`.

```rock
double: I64 -> I64
double = value -> value * 2

main = !->
    double 5 .println!
```

The output is `10`. `double` is a named function value; the expression after the equals sign is still a lambda-shaped function body.

Pass a lambda directly when its purpose is local.

```rock
main = !->
    mut values = Vec::new!
    values.push 1
    values.push 2
    mapped = values.map value -> value + 10
    mapped[0].println!
    mapped[1].println!
```

`Vec::map` consumes `values`, moves each `I64` into the callback, and returns a new `Vec I64`. The output is `11` and `12`.

## Higher-Order Functions

A higher-order function accepts another function as a value.

```rock
apply: (I64 -> I64) -> I64 -> I64
apply = function, value -> function value

increment: I64 -> I64
increment = value -> value + 1

main = !->
    apply increment, 4 .println!
```

The parameter `function` has type `I64 -> I64`, `value` has type `I64`, and the result is `I64`. The output is `5`.

Applying a capturing lambda uses the same function type at the call site, but the value also carries its captured environment.

```rock
apply: (I64 -> I64) -> I64 -> I64
apply = function, value -> function value

main = !->
    offset = 10
    add_offset = value -> value + offset
    result = apply add_offset, 5
    result.println!
```

`add_offset` captures `offset` by shared access because it only reads it. The output is `15`. The capture cannot outlive the value it refers to.

## Callable Bounds

The standard library expresses callback requirements with callable traits. Call a trait-bound parameter with ordinary function-call syntax: `function value`, or `function!` when there are no arguments. This works for functions, closures, and user-defined types implementing the callable traits.

| Bound | How a call uses the callable |
| --- | --- |
| `Fn Args, Ret` | Borrows it through shared access. |
| `FnMut Args, Ret` | Borrows it through mutable access, allowing captured state to change. |
| `FnOnce Args, Ret` | Takes it by value, allowing captured owned values to be consumed. |

`Fn` implies `FnMut` and `FnOnce`; `FnMut` implies `FnOnce`. Choose the least restrictive bound your implementation needs: `FnOnce` for a single consuming invocation, `FnMut` for repeated invocations that may mutate state, and `Fn` when shared access is required. Custom callable implementations must also implement their supertraits.

```rock
apply_mut: M -> I64 -> I64 where M: FnMut I64, I64
apply_mut = mut function, value -> function value

main = !->
    mut calls = 0
    callback = value ->
        calls = calls + 1
        value + calls
    first = apply_mut callback, 10
    first.println!
    calls.println!
```

The callback's capture is mutable, so `apply_mut` calls it through `FnMut`. Its `mut function` parameter permits that mutable borrow. The outputs are `11` and `1`. The invocation itself borrows the callback, but passing the callback into `apply_mut` transfers ownership of this non-copy value.

Pass a mutable reference to keep using a mutable callback afterward. The same generic bound accepts `&mut F` when `F` implements `FnMut`.

```rock
twice: F -> I64 -> I64 where F: FnMut I64, I64
twice = mut function, value ->
    function value
    function value

main = !->
    mut count = 0
    mut callback = value ->
        count = count + 1
        value + count
    result = twice &mut callback, 10
    result.println!
    callback 10 .println!
```

The output is `12` followed by `13`. Each invocation reborrows the callable rather than moving it. Shared references similarly support callbacks implementing `Fn`. A callback that consumes an owned capture only supports `FnOnce` and cannot be invoked again after it is moved.

The explicit `.call`, `.call_mut`, and `.call_once` methods remain available, but ordinary calls select the appropriate protocol automatically. Callable bounds use `()` for no arguments, the argument type for one argument, and a tuple of argument types for multiple arguments; explicit methods receive that argument pack as one value.

## Call Holes

An underscore in a call argument creates a function waiting for that argument. The holes are filled from left to right.

```rock
combine: I64 -> I64 -> I64 -> I64
combine = first, middle, last -> first + middle + last

main = !->
    add_ends = combine 10, _, 30
    answer = add_ends 2
    fill_middle = combine _, 5, _
    second_answer = fill_middle 1, 9
    answer.println!
    second_answer.println!
```

`combine 10, _, 30` creates `middle -> 10 + middle + 30`, so `answer` is `42`. `combine _, 5, _` creates a two-argument function, so `second_answer` is `15`. Use a named lambda when the hole order would obscure ownership or evaluation order.

## Curried Functions

`~>` declares a curried function. Supplying fewer arguments returns a function for the remaining arguments.

```rock
add: I64 -> I64 -> I64
add = left, right ~> left + right

main = !->
    increment = add 1
    first = increment 2
    second = add 1, 2
    first.println!
    second.println!
```

`add 1` owns the captured `left = 1` in the returned function; `increment 2` supplies the remaining argument and returns `3`. The direct two-argument call also returns `3`, so the output is `3` and `3`.

## Operator Sections

An operator section is another compact function value. `(+ 2)` creates a function that adds `2` to its input.

```rock
increment: I64 -> I64
increment = (+ 2)

main = !->
    increment 5 .println!
```

The output is `7`. Use an explicit lambda such as `value -> 2 - value` when the direction of the operation is not visually obvious.

## Methods as Values

A zero-argument method name is a function value until `!` calls it. The value captures the receiver, so its lifetime follows the receiver's ownership.

```rock
struct Counter
    < value: I64

impl Counter
    @get: I64
    @get = -> @value

main = !->
    counter = Counter
        value: 21
    get = counter.get
    first = get!
    second = counter.get!
    first.println!
    second.println!
```

`get` is a callable value with a shared receiver. Both calls return `21`, and the output is `21` and `21`; the receiver remains available because `get` is shared. A mutable or consuming method value carries the corresponding borrow or move restriction.

## Current Limits

Function values obey the same ownership rules as explicit calls. A closure that borrows a local value cannot outlive that value, and a method value with a borrowed receiver cannot be detached from the receiver's lifetime. Use an owned capture when an API requires a callback to outlive the current borrow.
