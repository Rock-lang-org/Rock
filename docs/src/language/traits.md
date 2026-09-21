# Traits and Methods

A trait names a capability. A type implements the trait by providing the required methods and associated types, and generic callers can require that capability with a bound.

## Declaring and Implementing a Trait

This trait says that an implementor can calculate an area through a shared receiver. Two unrelated structs provide different implementations.

```rock
struct Circle
    < radius: I64

struct Rectangle
    < width: I64
    < height: I64

trait Shape
    @area: I64

impl Shape for Circle
    @area = -> @radius * @radius * 3

impl Shape for Rectangle
    @area = -> @width * @height

main = !->
    circle = Circle
        radius: 5
    rectangle = Rectangle
        width: 4
        height: 6
    circle.area!.println!
    rectangle.area!.println!
```

`circle.area!` selects the `Circle` implementation and returns `75`; `rectangle.area!` selects the `Rectangle` implementation and returns `24`. The output is `75` and `24`.

The `@` marker is part of the method contract. The trait signature omits an explicit `self` parameter because the receiver is implicit; the body can use `@radius`, `@width`, or `self`.

## Default Methods

A trait may provide a method body. An implementation can override one method and inherit the others.

```rock
struct Dog
    < name: I64

struct Cat
    < name: I64

trait Animal
    @speak: I64
    @speak = -> 0
    @legs: I64
    @legs = -> 4

impl Animal for Dog
    @speak = -> 1

impl Animal for Cat
    @speak = -> 2

main = !->
    dog = Dog
        name: 10
    cat = Cat
        name: 20
    dog.speak!.println!
    cat.speak!.println!
    dog.legs!.println!
    cat.legs!.println!
```

`Dog` and `Cat` override `speak!` but inherit `legs!`. The output is `1`, `2`, `4`, and `4`.

## Inherent Implementations

An inherent implementation adds methods directly to a type without defining a trait. Associated functions have no receiver and are called through the type path.

```rock
struct Point
    < x: I64
    < y: I64

impl Point
    new: I64 -> I64 -> Point
    new = x, y ->
        Point
            x: x
            y: y

    @sum: I64
    @sum = -> @x + @y

main = !->
    point = Point::new 3, 4
    point.sum!.println!
```

`Point::new` is an associated function and returns a `Point`; `sum!` is a shared method. The output is `7`.

## Receiver Ownership

The receiver marker also controls ownership.

```rock
struct Counter
    < value: I64

impl Counter
    @read: I64
    @read = -> @value

    ^@set: I64 -> ()
    ^@set = value ->
        self.value = value
        return

    ~@take: I64
    ~@take = -> self.value

main = !->
    mut counter = Counter
        value: 1
    counter.read!.println!
    counter.set 9
    counter.read!.println!
    counter.take!.println!
```

`@read` shared-borrows, `^@set` requires a mutable binding and writes in place, and `~@take` consumes the receiver. The output is `1`, `9`, and `9`; `counter` cannot be used after `take!`.

## Associated Types

An associated type lets an implementation choose one output type for a trait.

```rock
trait Projector
    type Output
    @project: Self::Output -> Self::Output

struct Identity

impl Projector for Identity
    type Output = I64
    @project = value -> value

main = !->
    identity = Identity
    result: I64 = identity.project 13
    result.println!
```

The implementation defines `Identity::Output = I64`, so `project` accepts and returns `I64`. Omitting the required associated type is a compile-time error.

## Generic Trait Bounds

A generic function can use a trait method or operator when its signature names the bound.

```rock
struct Box T
    < value: T

same_box: Box T -> Box T -> Bool where T: Eq
same_box = left, right -> left.value == right.value

main = !->
    first = Box
        value: 7
    second = Box
        value: 7
    same_box first, second .println!
```

The call specializes `T` to `I64`, and the `Eq` implementation for `I64` supplies `==`. The output is `true`. A bound is checked at the call site as well as in the generic body.

## Focused Standard Traits

The standard library uses the same mechanism for `Show` and `Clone`. This type implements both: `show!` creates display text, while `clone!` creates independent owned data.

```rock
struct Label
    < text: String

impl Show for Label
    @show: String
    @show = -> @text.clone!

impl Clone for Label
    @clone: Label
    @clone = ->
        Label
            text: @text.clone!

main = !->
    original = Label
        text: String::from_str "Rock"
    copy = original.clone!
    original.println!
    copy.println!
```

The output is `Rock` twice. Cloning the `String` gives each `Label` its own allocation, while `Show` borrows the receiver. Other focused standard traits follow the same selection model: [Arrays, Slices, and Tuples](arrays-slices-tuples.md) demonstrates `Index` and `IndexMut`, and [Functions as Values](../functional/function-values.md) demonstrates `Fn`, `FnMut`, and `FnOnce` with complete callbacks. Keep a trait small; combine bounds only when one operation genuinely needs several capabilities.

## Coherence and Current Limits

Rock requires one unambiguous applicable implementation for a trait call. The following exhaustive program is intentionally rejected because `Tag` has two implementations of the same trait:

<!-- compile-fail: overlapping impls -->
```rock
trait Named
    @name: &Str

struct Tag

impl Named for Tag
    @name = -> "first"

impl Named for Tag
    @name = -> "second"

main = !->
    tag = Tag
    tag.name!.println!
```

Rock does not choose `first` or `second` by declaration order; remove one implementation to make the call coherent. Missing associated type definitions are rejected for the same reason that missing methods are rejected: an implementation must satisfy its complete trait contract. Trait selection also preserves ownership, so it cannot turn a borrowed receiver into an owned value or make an owned value copyable.
