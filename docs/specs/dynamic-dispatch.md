# Dynamic dispatch, runtime generics, and existential types

**Status:** full-feature specification with implementation in progress; borrowed dispatch, selected erased-generic/consuming-owner paths and bounded sized Type-kind packages have execution coverage. Broader borrowing/HKT support, owning object openings, destination construction and checked equality remain incomplete. See [development progress](#17-development-progress-and-continuation) to resume work.

**Date:** 2026-09-29.

**Scope:** the complete object model described here is one feature commitment. Implementation may proceed through dependency-ordered changes, but borrowed-only dispatch, missing generic methods, or missing ownership support do not satisfy this specification.

## Contents

- [Decisions and scope](#1-purpose-and-decisions)
- [Existing implementation](#2-existing-implementation-findings-and-implications)
- [Object types and syntax](#3-object-types-and-syntax)
- [Method selection](#4-method-selection-and-eligibility)
- [Supertraits and upcasting](#5-supertraits-multiple-guarantees-and-upcasting)
- [`Self` and existentials](#6-self-and-existential-operations)
- [Borrowing and effects](#7-borrowing-effects-and-hidden-origins)
- [Ownership and allocation](#8-ownership-destruction-and-allocation)
- [Runtime representation](#9-runtime-representation-and-dispatch-abi)
- [Runtime generics](#10-runtime-generic-execution)
- [Compiler contracts](#11-compiler-representation-and-phase-ownership)
- [Artifacts](#12-separate-compilation-and-artifacts)
- [Diagnostics and tooling](#13-diagnostics-and-tooling)
- [Acceptance criteria](#14-acceptance-criteria)
- [Integration and validation](#15-integration-map-and-validation-strategy)
- [Completion boundary](#16-completion-boundary-and-remaining-implementation-choices)
- [Development progress and continuation](#17-development-progress-and-continuation)
- [Additional worked examples](#appendix-a-worked-language-examples)

## 1. Purpose and decisions

Rock will support type-erased trait objects with vtable dispatch, without a `dyn` keyword. The design combines explicit ownership with runtime generic execution and scoped reasoning about hidden types.

The normative terms **must**, **must not**, and **may** describe requirements, prohibitions, and implementation choices. Syntax introduced below is proposed syntax, not a claim that the current parser accepts it. Code examples describe the intended completed language. Compiler records and ABI diagrams use explanatory notation rather than executable Rock.

The central decisions are:

1. A trait in a value-type position denotes an unsized existential object type. A generic parameter constrained by a trait remains generic.
2. References and owners determine storage and ownership. Merely mentioning a trait never allocates memory.
3. Ordinary dynamic object references use a data pointer and a metadata pointer.
4. Method selection identifies a canonical trait member at compile time; a vtable supplies its concrete implementation at runtime.
5. Generic methods remain callable dynamically through an erased generic ABI with explicit type metadata and dictionaries.
6. `Self` retains its same-concrete-type meaning. Existential opening, explicit packaging, and checked equality provide access without weakening that meaning.
7. Consuming receivers, supertraits, owned and borrowed upcasting, associated types, callable traits, and external artifacts are required completion scope.
8. Type erasure must preserve borrowing, move, destruction, thread-safety, and unsafe-call obligations.
9. The compiler recognizes protocols through canonical language-item identities, never stdlib source names.

This is inspired by Rust's ownership and trait objects, but is not Rust's dyn-compatibility model or binary ABI. In particular, generic virtual methods and the existential operations below deliberately exceed ordinary Rust trait-object capabilities.

### 1.1 Boundaries

This feature does not introduce inheritance of storage, garbage collection, exception unwinding, runtime source compilation, JIT compilation, or a stable plugin/FFI ABI. Runtime dictionaries contain previously compiled code. Existing explicit external-dependency loading remains authoritative.

Not every method can be called with arbitrary arguments merely because a receiver is an object: `Self` equality, bounds, borrowing, and receiver ownership remain real constraints. Those constraints are part of a complete feature, not missing dispatch support.

### 1.2 Architectural implementation standard

Existing abstractions are starting points, not constraints on the correct design. When an abstraction cannot represent this feature's semantics cleanly, implementation must refactor or replace it and migrate its callers. A larger coherent change is preferred to a smaller patch that creates a second source of truth or weakens an invariant.

In particular:

- Unify duplicated type/layout, trait-evidence, and ABI rules before extending them with dynamic cases. Do not independently teach inference, selection, MIR, and LLVM competing interpretations of the same fact.
- Model new distinctions explicitly: unknown versus unsized, concrete versus descriptor-backed layout, payload destruction versus owner release, and a selected member versus its runtime implementation.
- Rework body production, ownership analysis, or callable contracts when their existing shapes assume monomorphized concrete code. Do not route erased execution around their validation.
- Keep semantic authority in the responsible compiler phase. Do not add downstream rediscovery, name-based exceptions, extra metadata sidecars, or casts that conceal missing type/evidence relationships.
- Migrate all production callers of a replaced abstraction and delete the superseded path within the same coherent change. Do not preserve transitional aliases, repair fallbacks, or deprecated implementations merely to reduce diff size.
- Limit refactoring to responsibilities required by this feature or defects exposed by that work; unrelated cleanup is not a prerequisite.
- Preserve existing supported behavior through meaningful regression tests. When an intentional language/ABI change is required, update its specification, diagnostics, artifacts, and consumers together.

These rules apply to stdlib owners and allocators as well as compiler internals. `Box`, `Arc`, and their allocation protocols may be redesigned where necessary; adapting an unsuitable representation with special compiler knowledge of their source names is prohibited.

## 2. Existing implementation: findings and implications

The following observations come from the current checkout. Repository memory contains historical states; source is authoritative. `CLEAN_SLATE_COMPILER_AUDIT.md`, referenced by `MEMORY.md`, was not present during this analysis.

| Area | Current source and behavior | Required extension |
| --- | --- | --- |
| Pipeline | `lib/src/lib.rs::compile_impl` runs monomorphization, instance-body extraction, DCE, MIR construction, borrow checking, MIR agreement, then LLVM generation | Add erased body production and metadata reachability before MIR; validate both execution modes |
| Surface types | `lib/src/ast/tree.rs::ParseType` has references, pointers, applications, lambdas, projections, arrays, and tuples | Object qualifiers, existential binders, and new expression forms |
| Type lowering | `lib/src/type_lowering.rs::lower_inner_raw` resolves aliases and nominal constructors, otherwise obtains a generic type; traits are currently resolved separately for projections | Resolve trait names in value types explicitly; do not accidentally infer a generic parameter for a known trait |
| Semantic types | `lib/src/types/mod.rs::Type::Object` and `types/object.rs` represent principal trait, guarantees, and instantiated associated bindings | Complete declaration-aware object validation, existential binders, opened witnesses, and runtime parameter identities |
| Interned types | `lib/src/type_context/mod.rs::Ty::Object` uses the same object records with `TypeId` children | Complete semantic object admission and later runtime evidence support |
| Kinds | `lib/src/type_services/kind.rs` has `Type` and arrow kinds; existing code supports constructor application and type lambdas | Runtime constructor descriptors and kind-correct dictionaries; distinguish kind from sizedness |
| Method authority | `HirSelectedMethodTarget`, `HirMethodCallTarget`, and `HirTraitDispatchKind` in `lib/src/hir/mod.rs` preserve IDs and substitutions | Explicit object/dictionary dispatch authority, rather than unresolved generic fallback |
| Implied trait evidence | `lib/src/traits/evidence.rs::implied_trait_bounds` is shared by inference, bound/callable selection, and static-member lookup; member ambiguity uses `HirSelectedTraitMember`, including trait arguments | Canonical object signatures and producer-owned slot schemas must build on this evidence without treating traversal order as an ABI |
| Receiver adjustment | `lib/src/selection/types.rs` represents shared/mutable autoref, dereference, and slice adjustments | Preserve the same candidate machinery and record object coercions/upcasts explicitly |
| Trait interfaces | `HirTraitFor` carries predicates, associated types, methods, and signatures; `ProductTraitInterface` persists these | Stable slot schemas, supertrait graph, generic ABI and ownership summaries |
| Inference | `lib/src/infer/authority.rs` resolves delayed method/callable authority | Expected-type object coercions, existential escape checks, dictionary obligations |
| Runtime layout | `lib/src/type_services/layout.rs` recognizes slice/string fat pointers and rejects unresolved generic runtime types | Metadata kinds and a separate validated erased-body layout model |
| Sizedness | `lib/src/type_services/layout.rs::TypeLayout::sizedness` distinguishes sized, unsized, and unknown; inference and selection supply their available generic evidence | Extend this shared authority for objects, runtime-sized parameters, and unsized nominal tails |
| MIR | `lib/src/mir/mod.rs` has concrete places, operands, calls, drops, and ordinary rvalues | Typed metadata operations, virtual calls, dynamic storage and ownership transfers |
| Backend authority | `lib/src/mir/backend_contract.rs::MirBackendContract` owns callable declarations, layouts, projections, drop glue, exports, and runtime requirements | Object schemas, dictionaries, descriptors, erased signatures, owner adapters |
| Borrowing | `lib/src/mir/borrowck/mod.rs` tracks reference origins and return summaries; missing summaries can be conservative | Interface-level dynamic-call summaries and hidden-payload origin retention |
| Indirect calls | `lib/src/codegen/mir_llvm/terminator.rs` accepts nonconstant call operands of `Type::Function`; `codegen/types.rs` uses `{code, environment}` | Reuse LLVM indirect-call mechanisms, not the function-value semantic type, for object dispatch |
| Destruction | `lib/src/codegen/mir_llvm/terminator.rs` resolves concrete drop glue through the contract | Descriptor-driven payload drop, distinct from owner deallocation |
| Owners | `stdlib/box_type.rk` stores a raw pointer and byte count; `stdlib/arc.rk` points to `ArcInner T` | Metadata-aware owner protocols and allocation-base preservation |
| Allocation | Current owners pass `align: 1`; `stdlib/alloc.rk::Global` delegates to malloc/free/realloc and `checked_alloc` rejects zero size | Correct size/alignment and explicit zero-size behavior for dynamic payloads |
| Callables | `stdlib/callable.rk` defines the `FnOnce`/`FnMut`/`Fn` hierarchy with consuming/mutable/shared receivers | Dynamic callable objects and dictionary calls through the same protocol IDs |
| Protocol identity | `lib/src/language_items.rs` includes `Sized`, `Drop`, `Send`, `Sync`, and callable roles | Additional marker-bound owner/metadata protocols as needed |
| Artifacts | F03 introduces format **48** with portable object type rows; both version constants are updated | Complete declaration-aware object validation and later erased-body/evidence schema revisions |

Two consequences are architectural, not local LLVM changes:

- The existing requirement that ordinary final MIR contain no unresolved `Type::Generic` must remain. Runtime generics need a new, explicitly bound representation, not an exemption that admits arbitrary unresolved types.
- Current borrowed-reference lowering must continue to refer to the actual source storage. Object construction must not secretly heap-promote borrowed temporaries.

## 3. Object types and syntax

### 3.1 Trait names in type positions

| Form | Meaning |
| --- | --- |
| `Display` | Unsized object whose hidden concrete type implements `Display` |
| `&Display` | Shared borrowed object |
| `&mut Display` | Mutable borrowed object |
| `*Display` | Raw object pointer, including metadata |
| `Box Display` | Uniquely owned object, using the ordinary imported `Box` owner |
| `Arc Display` | Shared owned object, using the ordinary imported `Arc` owner |
| `T` with `T: Display` | Generic parameter, not automatically an object |

Name resolution, including ordinary shadowing and qualified names, determines whether a name denotes a trait. Unknown names must continue through the language's generic-name rules; a known trait must not take that fallback path. Type aliases to object types have identical semantics.

No `dyn` token is introduced. The normal generic trait-argument syntax is retained: `&Compare I64` refers to the object for `Compare I64`. Parentheses delimit nested applications when required by Rock's existing type grammar.

The following complete example requires no stdlib dispatch special case:

```rock
trait Value
    @value: I64

struct Number
    inner: I64

impl Value for Number
    @value = -> self.inner

read_value: &Value -> I64
read_value = item -> item.value!

main = !->
    number = Number
        inner: 42
    object: &Value = &number
    read_value object
```

### 3.2 Object qualifiers

An optional brace suffix binds associated types and adds guaranteed traits:

```text
object-type      := trait-application [ "{" qualifier ("," qualifier)* [","] "}" ]
qualifier        := associated-path "=" type | trait-application
associated-path  := member-name | qualified-trait-member
```

Braces here are type grammar, not a structural literal or a dynamically defined operator. A qualifier without `=` denotes an additional trait bound. The first trait is the principal view; additional ordinary traits and marker traits constrain the same hidden type. No `+` operator acquires compiler-owned trait-composition meaning.

- Trait arguments must be supplied or inferred before accepted HIR.
- Every associated type in the object's trait closure must have a unique normalized binding, either explicit or implied by predicates.
- A short associated name is legal only when unambiguous. Inherited collisions require a qualified trait-member path.
- Associated type constructors use existing kind-correct type expressions, including constructor holes and type lambdas.
- Duplicate equivalent qualifiers normalize away; inconsistent bindings are errors.
- Qualifiers do not enable runtime trait discovery: only guaranteed traits are available.

For example, this proposed program fixes an associated type explicitly:

```rock
trait Source
    type Item
    @get: Self::Item

struct NumberSource
    value: I64

impl Source for NumberSource
    type Item = I64
    @get = -> self.value

read_source: &(Source { Item = I64 }) -> I64
read_source = source -> source.get!

main = !->
    source = NumberSource
        value: 7
    read_source &source
```

### 3.3 Sizedness and storage

Object types have kind `Type` but are unsized. Kind correctness does not imply a compile-time layout.

- References and raw pointers to object types are sized fat pointers.
- An ordinary local, by-value parameter, return slot, array element, or non-tail field must be sized. Bare object types in those positions are errors.
- `Vec Trait` is invalid; `Vec (Box Trait)` is valid.
- Ordinary kind-`Type` generic parameters are sized by default. Trait `Self` is exempt from this default; individual operations may require a sized witness. The new bound form `T: ?Sized` relaxes the default requirement; it does not require `T` to be unsized. The `?` bound modifier applies to the trait path resolving to the sizedness language item, so aliases/renamings work; it is not an operator expression or a compiler test for the source string `Sized`.
- Owners and reference-oriented APIs must use relaxed bounds where appropriate. Existing generic declarations must be audited rather than assumed compatible.
- A record may have one unsized final field. Its other fields are sized, its alignment includes the tail alignment, and its metadata determines tail layout. Such a record is itself unsized and must be used indirectly.
- Unsized array elements, multiple unsized fields, and unsized enum payloads are rejected. Supporting dynamic objects does not make arbitrary aggregate layout possible.

For the standard two-word object representation, an erased concrete payload must be a sized value. Existing unsized slices/strings cannot be directly erased as payloads because they already need independent metadata. A sized wrapper containing their fat reference can implement the desired trait and be erased normally. This restriction must be diagnosed, not worked around by silently allocating a metadata box.

The `Sized` auto-rule for objects is false. The hidden witness of an ordinary object is sized, but its size is runtime-known outside its implementation. This distinction is essential for generic execution and existential opening.

Sizedness remains a compiler layout property even without a stdlib provider; spelling a trait bound that queries it uses the marker-bound language item when supplied. Constructor-kind parameters have no value layout themselves. Requirements that a fully applied constructor result be sized must travel with the generic layout obligations, not be inferred from its kind alone.

### 3.4 Coercions and inference

Object coercions are allowed at expected-type boundaries: annotated bindings, function arguments, returns, aggregate fields, and explicitly typed branch joins.

Supported transformations are:

1. `&T` to `&Trait` and `&mut T` to `&mut Trait`, given the complete implementation/binding evidence.
2. `&mut T` to a shared object reference through an ordinary shared reborrow.
3. Metadata-preserving owner unsizing, such as `Box T` to `Box Trait` or `Arc T` to `Arc Trait`, through the owner protocol.
4. Object upcasting and removal of guaranteed qualifiers.

No concrete value is implicitly boxed. No branch join invents an object type. An expression without an object expectation retains ordinary concrete/generic inference. Mutable coercions reborrow or move according to context; they do not duplicate exclusive access.

Coercions are directional relations, not type equality. Object signatures are invariant in trait arguments and associated bindings unless a separately justified variance rule exists. In particular, two objects with different associated bindings are not interchangeable.

Raw object-pointer construction and upcasting use the same evidence rules, but dereference and arbitrary reconstruction remain unsafe. A cast from a thin pointer or integer must not fabricate missing object metadata. The data-address component may be extracted explicitly under raw-pointer rules; converting it back requires the matching metadata and the usual provenance/validity obligations. Object handles and internal metadata must not cross an `extern` ABI by value without an explicit foreign representation or opaque-handle wrapper.

## 4. Method selection and eligibility

### 4.1 No blanket generic-method or `Self` ban

Traits remain usable as objects when they contain generic methods, consuming methods, defaults, associated functions, or methods involving `Self`. Availability is checked per operation, using the receiver and available evidence.

| Method shape | Requirement for a call |
| --- | --- |
| Shared `@` | Shared or stronger permitted receiver access |
| Mutable `^@` | Exclusive access to the payload |
| Consuming `~@` | Transferable unique ownership of the payload |
| Generic method | Type/constructor descriptors and dictionaries for its bounds |
| Input involving `Self` | An argument with the same opened witness, or checked equality evidence |
| Output involving `Self` | Preserve the witness; erase or package explicitly when it leaves scope |
| Associated function without receiver | An opened witness or explicit concrete type supplying the dictionary |
| Unsafe method | Existing unsafe-call authorization |

Trait-level predicates must be satisfiable. A higher-kinded trait target is not itself a value type: a bare reference to a constructor-level trait is a kind error. Constructor evidence is supported through runtime generic dictionaries and existential constructor binders instead.

### 4.2 Selection authority

Selection resolves `(trait ID, instantiated trait arguments, member ID)` before MIR. The selected receiver adjustment and all method substitutions are retained. Dynamic dispatch never performs string-based method lookup.

The normal candidate rules still apply. Inherent methods on an owner do not become methods of its payload; dereference and receiver adaptation are explicit selection steps. A collision between unrelated guaranteed traits produces an ambiguity diagnostic and uses existing qualified trait-member selection to disambiguate.

The object view exposes its guaranteed trait operations, but that does **not** universally imply `TraitObject: Trait`. Automatic forwarding evidence for an unsized generic argument is allowed only when the trait predicates and every member signature can be honored with the object view as `Self`. In particular, an argument of `&Self` cannot be forwarded as if two arbitrary object payloads shared a concrete type, and a receiver-free constructor cannot choose an implementation without witness evidence.

For forwarding-compatible traits, a generic receiver with `T: ?Sized` may accept the object view using a compiler-provided forwarding dictionary. For other traits, callers must open the object and pass its hidden witness/access path into generic code. This distinction preserves generic trait promises without imposing a blanket restriction on forming or directly using the object itself. Forwarding evidence is not a user-written impl that overlaps concrete impls.

### 4.3 Defaults and associated functions

At vtable/dictionary construction, each member resolves to its concrete override or the applicable trait default. Defaults preserve normal `Self` dispatch and must call overridden members correctly. Default code may be shared through dictionaries or specialized; behavior must agree.

An associated function cannot be invoked using only the trait name when choosing a hidden implementation would be ambiguous. Inside an opened scope, qualified calls through the witness are legal because its dictionary identifies an implementation.

### 4.4 Method values and callable traits

A bound method value captures the required receiver access and dispatch metadata. Shared/mutable method values retain loans; a consuming method value takes ownership. Existing `Fn`, `FnMut`, and `FnOnce` capability rules apply.

Generic methods can be instantiated when converted to an ordinary function value. A method whose type parameters remain undetermined cannot be converted to the existing monomorphic function-value ABI. To retain a generic operation for later instantiation, retain its object/dictionary with a generic member; this specification does not add an independent rank-polymorphic function-pointer syntax.

Dynamic callable objects must support normal application syntax through marker-bound callable traits. `Fn` to `FnMut` to `FnOnce` follows the supertrait graph, but upcasting does not grant permission to consume a borrowed receiver. Associated `Output` bindings must remain consistent with callable result arguments.

## 5. Supertraits, multiple guarantees, and upcasting

Build the trait closure from canonical predicates on the trait's `Self`, with trait-argument substitution applied at each edge. Do not discover inheritance by matching source names.

- Reject cycles with the actual predicate path in the diagnostic.
- Deduplicate the same instantiated ancestor reached through a diamond.
- Distinct instantiations of the same trait remain distinct views; unqualified ambiguous calls are errors.
- Validate associated-type equalities across the entire closure.
- A principal view with additional trait guarantees carries links to those views too.

Upcasting is allowed whenever the destination view and qualifiers are entailed by the source view. It may change the metadata pointer; it must preserve the payload pointer, provenance, ownership, and applicable borrow restrictions. Required reachable view projections must be prepared during metadata construction, so an upcast itself does not instantiate an allocating metadata factory.

Required cases include shared references, mutable reborrows, raw pointers under unsafe rules, unique owners, and shared owners. Upcasting must not allocate, move the payload, reset reference counts, or substitute the base trait's destructor for the concrete destructor.

The implementation must use explicit view links, not assume all supertrait tables are prefixes. Owned upcasting is a metadata transformation through the owner protocol. Implicit downcasting or recovery of dropped guarantees is not allowed.

## 6. `Self` and existential operations

### 6.1 Hidden identity

An object is conceptually a package of a sized witness type `X`, a value/access path involving `X`, and evidence that `X` satisfies the object signature. Two independently obtained objects have independent witnesses, even when their visible trait signatures are equal.

`Self` in a method refers to that witness, not to the visible trait type. In particular:

- `&Self` is not an arbitrary reference to another implementation of the trait.
- `Vec Self` is not `Vec Trait` or `Vec (Box Trait)`.
- `Box Self` retains its concrete payload destructor and allocation contract.

### 6.2 Scoped opening

Introduce an expression with an indented body:

```text
open expression as TypeBinder, value_binder
    body
```

The expression is evaluated once. It must be an object handle or explicit existential package. The compiler introduces a fresh rigid witness and exposes the corresponding handle or payload under `value_binder`.

- Opening `&Trait` gives `&X`; opening `&mut Trait` gives `&mut X`.
- Opening `Box Trait` by value gives `Box X` and moves the original owner.
- To preserve an owner, open a borrowed view of its payload.
- Opening an explicit package exposes its declared payload with its binder replaced by `X`.
- The witness cannot unify with an arbitrary inference variable or escape its scope unbound.
- The block can return an ordinary independent result, an object-coerced handle, or an explicitly repackaged existential.
- Opening does not allocate, clone, downcast, or extend any lifetime.

For example, this program uses a `Self` result as another method's `Self` argument:

```rock
trait Rebuild
    @rebuild: Self
    @accept: &Self -> ()

struct Token
    value: I64

impl Rebuild for Token
    @rebuild = -> Token
        value: self.value
    @accept = other !-> ()

exercise: &Rebuild -> ()
exercise = object !->
    open object as T, value
        rebuilt: T = value.rebuild!
        value.accept &rebuilt

main = !->
    token = Token
        value: 3
    exercise &token
```

`rebuilt` has runtime-known storage inside the open scope. It is not a bare unsized trait local: it is a value of a rigid sized witness with a layout descriptor.

### 6.3 Ordinary object calls involving `Self`

Calls whose public result does not mention the witness behave normally. For witness-dependent results:

- A borrowed `&Self`/`&mut Self` result may coerce to the appropriate object view, retaining its origin and access mode.
- An owned `Box Self` result may coerce to the corresponding object owner without reallocating.
- A bare `Self` result requires an opened scope or explicit destination packaging.
- More complex results, such as `Vec Self`, require opening or explicit existential packaging; containers are never rewritten element-by-element implicitly.
- Results do not inherit extra trait qualifiers unless those guarantees are proven for the same witness.

An implicit opening used for one call is confined to that expression. Code that needs to relate multiple results and arguments must use an explicit opening; it must not rely on receiver variable names as type identity.

### 6.4 Explicit destination packaging

Introduce the expression:

```text
pack (owner-type) (value-expression)
```

For example, the target may be `Box Rebuild` and the value expression may be a virtual method returning its hidden `Self`.

This operation explicitly requests owner storage. It evaluates the producer's receiver/callee and arguments in normal order, obtains the result layout from their evidence before entering the producer body, allocates through the selected owner's construction protocol, evaluates the producer body exactly once into that destination, and commits initialization on normal return. The witness must satisfy the target object's complete signature. For an already evaluated value, its known descriptor supplies the layout and the value is moved into the destination once.

Packaging an existing owned handle by coercion does not require `pack`; using `pack` with an already owned handle instead packages that handle as the payload if its type satisfies the requested trait. The compiler must not silently interpret the two operations interchangeably.

The owner protocol supports destination construction so a runtime-sized result need not be created on the stack and copied into the heap. Allocation failure follows the selected owner's policy; the standard global owner follows the stdlib's process-termination policy. There is no exception-unwind cleanup requirement.

### 6.5 Explicit existential packages

To retain relationships beyond an open scope, introduce a type binder:

```text
exists T where T: Trait in PayloadType
```

The binder also accepts the existing constructor-kind parameter forms, such as `F _`. Multiple related values can be held in a tuple payload. For example, the type-level form `exists T where T: Trait in (Box T, Box (Vec T))` preserves one common witness across both owners.

Construction uses:

```text
pack (existential-type) (payload-expression)
```

Here `pack` packages existing payload values and their evidence; it does not request owner allocation. This is distinguished by the target type category. The witness substitution is inferred from the payload and must be unique; otherwise an opened/annotated payload is required.

An existential package is a sized value only if its payload has a uniform compile-time layout independent of the hidden witness. References over sized witnesses and owner types verified to store their payload indirectly satisfy this condition; an arbitrary custom owner is not assumed uniform. A direct hidden `T` is not uniform. Payload types whose pointer metadata widths depend on an unknown sizedness are likewise not uniform without another owner boundary.

Thus an existential over `Box (Vec T)` is legal; an existential storing `Vec T` inline requires proof of uniform layout and cannot be assumed valid merely because current `Vec` happens to use pointer fields. Layout uniformity is a structural compiler fact, not a user assertion.

Package representation consists of its uniformly laid-out payload and the descriptors/dictionaries necessary to reopen it. It need not be two words. Closing a scope does not erase its hidden borrow obligations: they remain attached to the package value.

### 6.6 Checked equality of independent witnesses

Introduce a scoped conditional form:

```text
open left_expression, right_expression as T, left, right if same_type
    success_body
else
    failure_body
```

Both expressions are evaluated exactly once, left to right. They must provide object handles or single-witness packages whose hidden witnesses are eligible for comparison. In the success branch, both witnesses are represented by `T` and their dictionaries remain available. In the failure branch, `left` and `right` retain their original independent package types; no `T` is in scope. Moving owner expressions moves them on either branch, and their destruction follows the chosen branch normally.

Equality is checked against canonical concrete type identities, not vtable addresses, function addresses, layout sizes, or source names. Different trait views of the same concrete type can compare equal.

**Runtime type identity is not lifetime equality.** Before introducing full type-equality evidence, the compiler must establish compatibility of hidden borrow/region contracts, including invariant nested references and constructor parameters. A runtime identity comparison alone may not unify different reference lifetimes or make a borrowed payload borrow-free.

For independently erased values with insufficient static region information, the checked opening must report that type-equality evidence cannot be established. Borrow-free payloads and witnesses already sharing a compatible region abstraction support the check. This is a soundness condition, not permission to implement an unchecked downcast.

The equality witness is scoped and cannot be forged, stored as a Boolean for later casts, or used outside its successful branch. General named-type downcast convenience APIs may reuse this mechanism, but are not necessary to express same-`Self` calls.

## 7. Borrowing, effects, and hidden origins

The representation's data pointer does not replace the semantic access path. Object construction, opening, upcasting, packaging, method values, and generic calls must participate in the same loan and move analysis as concrete values.

### 7.1 Required invariants

1. Shared object references cannot mutate or consume the payload.
2. Mutable object references are not copied; upcasts create reborrows or transfer the reference.
3. Borrowed objects cannot outlive their source places.
4. Owning an erased payload does not prove that its contents are borrow-free.
5. Erased closures and method values retain capture origins, including across control-flow joins.
6. Hidden references inside a `Self` result or generic result retain their origin constraints after packaging.
7. A moved receiver, payload, or existential package is invalidated on every relevant path.
8. Metadata pointers carry no permission to access payload storage after its owner or loan expires.

### 7.2 Dynamic-call borrow contracts

Concrete implementation return summaries cannot be the caller's authority: implementations can live in separate artifacts and the runtime choice is unknown.

Each trait member and erased callable interface must carry a symbolic borrow contract describing possible origins of returned references and references nested in returned owned values, and possible retention of argument borrows in mutable destinations. Paths are structural argument/result paths, with opaque hidden-payload origin variables where fields are not visible.

The initial surface language need not add named lifetimes. Derive a conservative contract from the declared signature and predicates: potentially borrowing outputs are constrained by every type-compatible input origin and any hidden origins of the receiver/arguments. Borrow-independent outputs carry no such origins. Ambiguous multi-input borrowing may therefore retain more loans than a concrete call.

The same contract must cover writes and escapes, not just returns. Potentially borrowing arguments may be retained in type-compatible mutable receiver/argument storage only when their origins outlive that destination's origin envelope. Signature-only contracts conservatively include such possible retention; implementations may narrow behavior but cannot expand it. This prevents a virtual setter returning `()` from hiding a short-lived reference inside a longer-lived erased owner. Shared references grant no such write permission. Raw-pointer escapes remain governed by the existing unsafe contract.

Every implementation and default must be checked against that interface contract. It may return fewer origins, never a local origin or additional origin not authorized by the contract. Recursive summaries are computed to a fixed point and persisted in artifacts. Unknown summaries cannot be treated as borrow-free.

Local object formation retains concrete hidden-origin facts. Public object interfaces use abstract origin parameters that callers instantiate; serialized contracts do not contain references to a producer compilation's local variables. This is needed for containers, cross-crate calls, threads, and same-type checks.

### 7.3 Thread-safety guarantees

An object retains only the marker guarantees present in its signature or implied by its trait closure. `Send`/`Sync` semantics use their existing language-item roles and stdlib policies.

- Owned transfer requires the appropriate payload guarantee and any owner requirements.
- Shared access across threads requires the appropriate shared-access guarantee.
- An `Arc` does not itself make arbitrary payloads thread-safe.
- Thread-spawn escape checks must reject hidden borrowed captures even when their concrete type was erased.
- Removing qualifiers is allowed; adding them requires evidence, not inspection of a runtime implementation name.

## 8. Ownership, destruction, and allocation

### 8.1 Separate payload metadata from owner metadata

The concrete payload descriptor provides layout and in-place destruction. It does not, by itself, know how its enclosing owner allocated storage.

- `Box` can use the payload layout for its ordinary allocation.
- `Arc` also needs its allocation base, reference-count control block, payload offset, and complete allocation layout.
- An arena/custom allocator may need allocator state and a different release operation.

Consequently, dropping a trait object means invoking the owner's drop implementation, which invokes payload drop when appropriate and releases storage through the owner's protocol. A generic vtable `free(data)` entry would be incorrect.

### 8.2 Required owner protocol operations

Compiler-visible protocol roles must be declared by language items and checked structurally. Standard owner source names are not compiler identities. The protocol must express:

| Operation | Obligation |
| --- | --- |
| Pointee view | Expose the exact data pointer, metadata, and shared/mutable capability |
| Metadata replacement | Change a view while preserving allocation and owner state |
| Destination construction | Allocate/reserve correctly aligned storage, initialize exactly once, and commit ownership |
| Construction cancellation | Release an uncommitted allocation on normal early exit, dropping only initialized payload parts |
| Unique extraction | Transfer the payload out and release now-uninitialized backing storage without dropping it twice |
| Destruction | Drop initialized payloads exactly once and release allocation through the correct allocator |

Unsafe protocol implementations must satisfy these invariants; safe callers cannot construct arbitrary pointer/vtable pairs. Custom owners implementing the same validated roles must work without compiler changes keyed to their names.

Construction cancellation requires canonical authority distinct from extracting an initialized owner's payload. A shared owner may support allocation, cancellation, and commit without permitting consuming dispatch or general unique extraction; construction must not obtain cleanup by fabricating that stronger capability. Commit uses the validated metadata/state reconstruction operation only after payload initialization succeeds.

The approved construction protocol adds cancellation to the existing allocation role:

```rock
lang object_owner_allocate
< trait ObjectOwnerAllocate State for F _
    lang owner_allocate
    unsafe allocate: I64 -> I64 -> (*U8, State)
    lang owner_cancel
    unsafe cancel: *U8 -> State -> ()
```

The allocation arguments are byte count and alignment. Cancellation consumes the unpublished allocation's exact pointer and State, releases backing storage and initialized owner-internal resources, and never reads or destroys payload bytes. Required backing layout/provenance belongs in State; cancellation needs neither a payload descriptor nor unique extraction authority. Commit instead consumes State through canonical `from_parts` and disarms cancellation. On normal exits bypassing commit, MIR construction/drop elaboration first destroys only still-initialized payload parts (or a fully initialized unpublished payload), then cancels storage exactly once. A normal producer return into its destination is successful initialization, not cancellation.

### 8.3 Standard owners

`Box` must support construction, borrowing, mutable access, coercion, upcasting, drop, and consuming dispatch for object payloads.

`Arc` must support construction, cloning, shared borrowing, coercion, upcasting, and final-reference destruction. It must not expose mutable or consuming dispatch simply because the underlying method exists. A checked unique-extraction API can produce a unique owner when the reference count permits; direct `~@` dispatch on an arbitrary shared owner is rejected.

Supporting unsized tails permits the existing control-block style to evolve, but an implementation may use separate header/payload pointers instead. In either case, data-pointer upcasting must not lose the original allocation base.

### 8.4 Consuming receivers

For `~@` through a unique owner:

1. Verify no incompatible loans remain and move the owner into the operation.
2. Obtain the concrete payload storage and the owner's unique-extraction/release authority.
3. Transfer initialization responsibility for the payload to the consuming adapter/body.
4. Execute the method; its normal move/drop behavior owns any remaining payload fields.
5. Release the original backing allocation without running a second payload destructor.

The body may move the value elsewhere or deliberately forget it. Releasing vacated backing storage is distinct from dropping the transferred value. Returning a value related to the consumed receiver must not leave references into released storage.

A receiver of type `Box Self` in an explicitly declared signature is different from a `~@` receiver: the former receives the owner as a value. Adapters must preserve that distinction.

### 8.5 Layout correctness

Dynamic layout uses target-derived size, alignment, stride, and overflow-checked offset arithmetic. All allocator operations must honor alignment. The current `align: 1` convention is not sufficient evidence of correctness for the new paths.

Zero-sized payloads are supported with a non-null, appropriately aligned sentinel or valid owner-specific storage; destruction still runs when required. They must not be sent to an allocation helper that unconditionally rejects size zero. Over-aligned layouts require an allocator path that actually satisfies their alignment.

Initialization flags must survive early returns, branches, partial aggregate construction, and consuming calls. Rock's existing abort/no-unwind behavior remains unchanged; normal control-flow cleanup remains mandatory.

## 9. Runtime representation and dispatch ABI

This section defines logical records. Physical padding and target-sized integer widths come from target layout. The ABI is internal and versioned with artifacts.

### 9.1 Object handles

```text
ObjectReference = { data: pointer, view: pointer<ObjectView> }

ObjectView = {
    concrete_descriptor: pointer<TypeDescriptor>,
    object_schema: ObjectSchemaIdentity,
    associated_evidence: [...],
    related_views: [...],
    method_entries: [...]
}

TypeDescriptor = {
    concrete_identity,
    size,
    alignment,
    stride,
    drop_in_place,
    structural_layout_evidence
}
```

The payload descriptor supplies the logical drop/size/alignment header discussed in the design. Factoring it into a shared descriptor avoids duplicating it in every trait view. No Rust byte-layout compatibility is promised.

Every operation entry is a typed code pointer plus an optional immutable evidence environment. For a fully specialized method the environment may be omitted; a runtime-instantiated impl or drop routine uses it to access its enclosing type/constructor arguments. These environments contain metadata, not borrowed payload storage, and follow the metadata lifetime rules in section 10.6.

Borrowed handles are two machine words. Owners retain whatever additional state their ordinary representation requires; they are not universally promised to be two words. Existing `{code, environment}` callable values remain a distinct ABI.

### 9.2 Identity and ordering

An object schema key includes canonical principal trait identity, normalized trait arguments, associated bindings, and additional guarantees. A concrete view key additionally includes the concrete type and implementation evidence.

Slot order must be deterministic and stored in the producer's schema. Use producer-local stable member identities and explicit schema indices; neither hash-map iteration nor importer-assigned crate numbers may determine ABI order. Imported schemas are validated and preserved through ID remapping.

Concrete type identity includes defining crate identity, nominal definition, type arguments, structural constructors, and relevant ABI identity. A hash may accelerate comparison but cannot be the sole unchecked proof if collisions are possible. Metadata address equality is only an optimization, not semantic equality. Lifetimes are not validated by this runtime key; section 6.6 supplies the additional rule.

### 9.3 Ordinary virtual calls

The canonical slot ABI uses an erased payload address plus ABI-described arguments and result storage. An adapter bridges this ABI to a concrete method's existing parameter/return conventions. Concrete scalar fast paths are permitted when all producers and consumers agree through the schema.

Dispatch steps are:

1. Evaluate receiver and arguments in Rock's normal order, once each.
2. Apply validated receiver adjustments/reborrows.
3. Load the method entry identified by the accepted call authority.
4. Provide the data pointer, required evidence, and result destination.
5. Make an indirect call with the exact contract signature.
6. Update move/initialization state and expose the result under its borrow contract.

Slot tables may use immutable globals for fully concrete views. Runtime-parameterized views use immutable instantiated metadata whose lifetime must cover every escaping handle.

## 10. Runtime generic execution

### 10.1 Two execution modes

The compiler supports specialized bodies and erased bodies:

- A specialized body has concrete runtime layouts and follows ordinary monomorphization.
- An erased body binds explicit runtime type/constructor parameters and receives their layout and operation evidence.

Erased execution is ahead-of-time compiled. It must not require knowing every caller's future type arguments or recompiling a producer artifact. Specialization and devirtualization are optional optimizations that preserve semantics.

Generic virtual methods have one erased entry per method schema, not an unbounded collection of type-specialized slots. Exported generic functions needed by erased code must expose an erased entry or accepted body sufficient to generate one. Exported generic APIs must be prepared for erased callers even when the producing crate has none.

### 10.2 Erased call shape

```text
erased_entry(
    implementation_environment,
    receiver_address_if_present,
    type_or_constructor_evidence[],
    bound_dictionaries[],
    argument_addresses[],
    result_address
) -> void
```

This is a logical signature; the contract expands it into a fixed LLVM function type for each schema. Argument count, evidence order, ownership mode, and result shape are known statically. It is not an untyped variadic runtime convention.

- Sized generic values are passed by address; their layout comes from descriptors.
- Shared and mutable access retain distinct semantic modes even where LLVM uses the same pointer type.
- By-value arguments transfer initialization responsibility to the callee; caller-side ownership is invalidated once.
- The caller supplies uninitialized result storage of the required layout. Normal return initializes it exactly once.
- Unit and never-returning cases have explicit ABI handling; no arbitrary result bytes are invented.
- Borrow summaries and unsafe requirements are part of the signature contract.

### 10.3 Operations inside erased bodies

Erased code must support the same well-typed operations its declared evidence permits:

| Operation | Runtime lowering |
| --- | --- |
| Local storage | Aligned runtime-sized storage with scoped initialization/drop state |
| Move | Transfer initialized storage; invalidate source, never call clone implicitly |
| Copy | Only with statically available copy evidence; descriptor does not grant it automatically |
| Drop | Descriptor operation for the exact instantiated type |
| Nominal field/variant access | Producer-defined layout template evaluated using descriptors; typed field/variant identity retained |
| Arrays and tuples | Checked layout composition and element operations |
| Trait operation | Selected dictionary member with its own ABI contract |
| Generic function call | Erased callee entry with composed evidence |
| Closure | Environment layout/operations parameterized by captured evidence and origins |
| Pointer offset / size query | Descriptor size/stride rather than assumed byte or integer placeholder |

Primitive arithmetic remains explicit intrinsics or trait implementation bodies, not runtime operator-name lookup. Intrinsics must be classified as descriptor-lowerable, concrete-type-only with a compile-time proof, or ill-typed for the given generic body. A valid unconstrained generic body cannot rely on type-specific operations absent from its bounds.

Dynamic locals may use aligned scoped stack storage or compiler-managed scratch storage; the implementation must bound their lifetime and release storage on normal exits. Metadata required by escaped values must never point into these temporary frames. Storage strategy is not an implicit source-level `Box` conversion.

### 10.4 Dictionaries and evidence construction

A trait dictionary identifies the instantiated trait, its associated-type/constructor evidence, related trait dictionaries, and method entries. Object views are dictionaries with an erased payload witness; the underlying schema machinery should be shared.

Implementation selection still happens at compile time. For a blanket impl whose inputs are runtime parameters, generate a dictionary constructor for that selected impl, parameterized by the required evidence. Do not install a global runtime search over all impls.

Recursive evidence must use cycle-aware construction/interning. Truly nonterminating type normalization or unsatisfied recursive obligations are diagnosed, rather than causing unbounded descriptor construction.

### 10.5 Higher-kinded parameters

The existing HKT language is included, not silently excluded from generic virtual methods.

- A constructor descriptor carries its kind, captured parameters, and a compiled application operation.
- Applying it to descriptors produces the descriptor for the fully applied runtime type.
- Associated type constructors travel through dictionaries with their declared kinds.
- Dictionary constructors for constructor-level traits preserve existing `F _` bounds.
- Constructor holes/type lambdas normalize where possible; abstract runtime applications become descriptor applications, not unresolved LLVM types.
- Nominal runtime layouts retain their defining identity and template even when arguments are runtime-known.

For example, a generic virtual method using a callback bounded by `FnMut` or applying a constructor bounded by `Functor` must execute through evidence without enumerating all callbacks/constructors in the producer crate.

### 10.6 Metadata lifetime and caching

Fully concrete descriptors and dictionaries are immutable globals. Runtime-instantiated metadata uses process-lifetime canonical storage in the initial implementation, with synchronized, cycle-aware interning when threads can instantiate it concurrently.

This may retain one metadata record per distinct encountered instantiation for the process lifetime. It does not retain payload values or borrow permissions. The cost must be documented and tested; a future reclamation strategy may change it without changing language semantics.

No application-level reflection or impl lookup is implied by metadata interning. No descriptor may retain a stack pointer to a caller's temporary evidence array.

## 11. Compiler representation and phase ownership

### 11.1 Types and accepted HIR

Introduce typed identities and records for object signatures, existential binders/opened witnesses, dictionary schemas, runtime parameters, and evidence paths. Exact Rust names may follow local conventions; semantic distinctions must not be collapsed into strings or `TypeVar`.

The semantic and interned type systems must represent:

1. Object signature with trait IDs, arguments, bindings, and guarantees.
2. Existential binder and payload type, with alpha-equivalent identity.
3. Scoped rigid opened witness with kind, sizedness, bounds, and origin abstraction.
4. Erased-body parameter references bound by an explicit function/schema environment.

Update all visitors/folders, substitution, kind checking, projection normalization, type interning, display, equality/hashing, source references, product encoding, and ID remapping. Existing type-lambda binders may supply reusable machinery, but a type lambda and an existential have different introduction/elimination rules.

Accepted HIR records object coercions, selected view paths, virtual/dictionary call authority, opening/packaging, and equality refinements explicitly. It must contain no unresolved inference/recovery types or targetless dynamic method calls.

### 11.2 Body production and reachability

Extend the current monomorphization boundary into a body-production stage that can emit specialized and erased instances. Existing concrete instance identity remains usable; erased bodies require an identity including their schema and selected implementation environment.

Vtable entries, default methods, owner adapters, dictionary constructors, descriptor drop functions, generic entries, and metadata factories are reachability edges. DCE must account for them before pruning instances. An entry is live even if no ordinary direct call references it.

Exported tables and erased generic APIs establish roots required by downstream crates. Renaming display names must not change selection or accidentally remove reachable code.

### 11.3 MIR operations

MIR must retain typed operations equivalent to:

- Construct an object view from a concrete access path and evidence.
- Change an object view along a validated trait-closure edge.
- Extract data/metadata under an access capability.
- Invoke a virtual or dictionary member under an exact schema.
- Instantiate a descriptor/dictionary and apply a constructor descriptor.
- Allocate/release scoped dynamic storage and transfer initialization.
- Construct/commit an owner destination and uniquely extract an owned payload.
- Open/close existential evidence and refine equality on a control-flow edge.
- Perform dynamic payload drop through an exact descriptor.

Some operations can lower into existing MIR calls to contract-declared helpers, but their ownership and evidence relationships must remain checkable. Do not model virtual calls as unresolved `MirCallable::Method` or reuse ordinary casts to smuggle metadata.

Ordinary constant callable operands remain `MirCallable::Resolved(MirCallableKey)`. Runtime virtual targets use an explicit typed call-target form or dedicated terminator, with schema evidence instead of a fabricated globally resolved callee. Concrete slot adapters remain resolved callables.

### 11.4 Backend contract

Extend `MirBackendContract` as the sole backend authority with:

- Object/dictionary schemas and canonical slot indices.
- Concrete and parameterized view declarations.
- Type/constructor descriptors and layout templates.
- Erased callable signatures and evidence environments.
- Owner adapter declarations and ownership transitions.
- Symbolic borrow/effect summaries needed by MIR validation.
- Runtime metadata/scratch/allocation requirements derived from observed MIR.
- Artifact exports and canonical link identities for the new records.

Do not create a parallel codegen metadata sidecar. Codegen consumes records; it does not resolve trait impls, repair missing signatures, or infer owner behavior from names.

### 11.5 Agreement and borrow validation

Keep the strict concrete-body barrier. Add an equally strict erased-body validator:

- Every runtime parameter belongs to the enclosing evidence environment.
- Every dynamically laid-out place names an available descriptor expression.
- Every slot/member index belongs to its schema and exact signature.
- Every dictionary construction cites an accepted selected impl/default.
- Every origin summary, owner transition, and move mode matches the call.
- Evidence scopes dominate their uses; equality refinements cannot cross the failure edge.
- No inference variable, recovery type, free witness, or unbound constructor reaches LLVM.
- No unsized payload is loaded/stored using a fabricated concrete LLVM type.
- Required runtime helpers and metadata dependencies match the contract exactly.

Borrow checking must analyze both body modes. Checking only specialized instances would miss errors in exported erased entries.

### 11.6 LLVM lowering

Add descriptor-based place addressing, runtime layout evaluation, typed indirect calls, global/runtime view construction, and owner adapters under `lib/src/codegen/`.

Reuse current indirect-call builder operations and concrete ABI adapters where compatible. Do not conflate object handles with closures, slice metadata with vtable pointers, or semantic `Self` with `i8` merely because LLVM uses opaque pointers.

Optimizations may eliminate dispatch, adapters, metadata construction, or redundant packing when proven equivalent. Correctness must not depend on optimization level or whole-program visibility.

## 12. Separate compilation and artifacts

The feature must work when traits, implementations, callers, and generic argument types are all defined in different crates.

Persist and validate:

1. Object signatures, qualifiers, associated bindings, and existential type rows.
2. Trait closures, slot schemas, receiver modes, generic kinds, and borrow contracts.
3. Erased generic entry declarations and required accepted body providers.
4. Descriptor/layout templates and dictionary constructors.
5. Vtable/adapter/drop/evidence symbols through product link records.
6. ABI/schema version, target layout identity, and dependency freshness information.

Producer schemas determine slot order. Importer ID remapping must cover every nested trait/member/type/binder reference without re-sorting slots by local crate IDs. Source/display names remain diagnostic metadata.

Object-backed generic entries must either be exported with sufficient hidden evidence inputs or have the body provider required to generate them. A downstream caller must not guess an unexported symbol or require producer source files.

Artifacts with incompatible schema/target ABI or malformed evidence references are rejected before codegen. F03 advances the artifact format from 47 to **48** in both `rock-lib` and `rock-shared`. Coordinate subsequent incompatible schema changes between both crates. No old-format compatibility shim is required.

The compiler must not discover or load stdlib implicitly for this feature. A no-stdlib program can define ordinary object traits; owner/marker protocols are available only when their providers are explicitly supplied. Missing required protocol roles produce structured diagnostics.

## 13. Diagnostics and tooling

All diagnostics use original source ownership and exact spans. Internal metadata must retain enough source identity to point to both the failing use and relevant declaration.

Required diagnostic categories include:

| Condition | Required explanation |
| --- | --- |
| Bare object in sized storage | Identify the storage position and suggest an explicit reference or owner |
| Missing/conflicting associated binding | Identify the associated member and conflicting trait paths |
| Missing object implementation | Show the concrete type and instantiated trait requirement |
| Invalid receiver capability | Distinguish shared, mutable, and consuming requirements |
| Generic bound missing | Identify the method parameter and missing dictionary obligation |
| Different `Self` witnesses | Explain that the same trait does not imply the same concrete type; point to opening/checking |
| Witness escape | Identify the binder and escaping type path; suggest repackaging |
| Region-incompatible equality | Explain why runtime type identity cannot prove lifetime compatibility |
| Unsized concrete erasure | Explain the extra metadata requirement and explicit-wrapper option |
| Invalid upcast | Show the missing trait guarantee or differing associated binding |
| Unsafe virtual call | Preserve ordinary unsafe-call diagnostics |
| Invalid artifact schema | Identify the artifact and schema/identity disagreement |

Parser, formatter, macro AST/token traversal, tree-sitter grammar/highlighting, and LSP analysis must recognize the new syntax. Type/member navigation must still use canonical definitions. Hover text must distinguish object types, scoped witnesses, and statically generic parameters, including their visible bounds.

New contextual syntax consists of object qualifier braces, `?Sized`, `exists ... where ... in ...`, `open ... as ...`, the `if same_type` opening guard, and `pack (...) (...)`. `open`, `pack`, `exists`, and `same_type` are contextual in these forms, not globally reserved identifiers. An opening consumes an indented block with the same result/early-exit rules as other block expressions; its optional `else` belongs to the guarded opening at the same indentation. `pack` requires the two parenthesized operands shown above. An existential binder extends through its complete payload type; parentheses delimit it inside a larger type application.

Qualifier braces bind to the immediately preceding trait application. Because Rock also uses commas inside trait applications, a generic trait qualifier must be parenthesized inside the brace list; a generic right-hand associated binding must likewise be parenthesized. Parser and formatter tests must preserve these boundaries rather than guessing comma grouping.

## 14. Acceptance criteria

These are completion requirements, not a claim that any listed behavior exists today. Tests must assert observable semantics or validator behavior rather than source-text presence.

### 14.1 Language and inference

| ID | Required evidence |
| --- | --- |
| L1 | Trait-as-type parsing, qualified names, aliases, qualifiers, and formatting round trips without `dyn` |
| L2 | The same generic-bound program remains statically generic; expected object types insert explicit accepted-HIR coercions |
| L3 | Shared/mutable calls across at least two concrete implementations return the correct results |
| L4 | Missing bindings, contradictory qualifiers, ambiguous members, wrong kinds, and sized-position violations fail with source spans |
| L5 | Generic method arguments infer from normal call/return context; no local annotation workaround is needed for inference regressions |
| L6 | Default methods call concrete overrides correctly through local and external objects |
| L7 | Generic forwarding accepts compatible object views but cannot turn a hidden `Self` equality requirement into arbitrary object equality |

### 14.2 Ownership and borrowing

| ID | Required evidence |
| --- | --- |
| O1 | Borrowed object construction/upcasting allocates no payload storage and preserves source addresses |
| O2 | Escaping local borrows, shared mutation, duplicated mutable access, and use-after-consume are rejected |
| O3 | Object owners containing hidden references retain escape restrictions through fields, branches, packages, and closures |
| O4 | `Box` drop and consuming calls destroy each initialized payload exactly once; moved/forgotten payload behavior remains correct |
| O5 | `Arc` clone/upcast preserves counts; final release destroys once; arbitrary mutable/consuming access fails |
| O6 | A custom owner proves the protocols do not depend on `Box`/`Arc` names |
| O7 | Zero-sized, alignment-sensitive, aggregate, and partial-initialization cases use correct layout and release paths |
| O8 | Borrowed method results and nested generic results obey interface origin summaries across artifacts |
| O9 | Thread transfer accepts only appropriate explicit guarantees and rejects hidden borrowed captures |
| O10 | A virtual/erased setter cannot store a short-lived input reference into longer-lived receiver storage, even when it returns unit |

### 14.3 Generic and existential behavior

| ID | Required evidence |
| --- | --- |
| G1 | One generic virtual method executes with several caller-chosen types absent from the producer crate |
| G2 | Erased methods call other generic functions and constrained callbacks without requiring concrete monomorphization |
| G3 | Runtime-sized locals, nested nominal layouts, tuples, enums, arrays, moves, and drops behave correctly |
| G4 | Constructor-generic virtual methods and associated constructors work through HKT dictionaries |
| G5 | `Self` return/input round trips work inside an opening; unrelated object inputs fail |
| G6 | Explicit owner packaging constructs bare `Self` results without hidden boxing elsewhere |
| G7 | Existential packages preserve one witness across multiple/nested payload values and reopen safely |
| G8 | Checked same-type opening succeeds across different views of one concrete type and takes the failure branch for different types |
| G9 | Equal layouts, duplicated vtables, hash collisions in a test identity provider, or incompatible hidden regions cannot forge type equality |
| G10 | Witness escape, missing evidence, stale initialization, and metadata pointing into dead frames are rejected or prevented |

### 14.4 Trait relationships and callables

| ID | Required evidence |
| --- | --- |
| T1 | Shared, mutable, and owned upcasts work through generic supertraits and diamond graphs |
| T2 | Repeated equivalent ancestors deduplicate; conflicting associated bindings and cycles are diagnosed |
| T3 | Additional ordinary and marker guarantees can be retained or forgotten without runtime impl search |
| T4 | `Fn`, `FnMut`, `FnOnce`, captured closures, and bound method values retain invocation/ownership behavior |
| T5 | Static associated functions are callable through an opened witness dictionary but not an ambiguous bare trait |

### 14.5 Backend and external artifacts

| ID | Required evidence |
| --- | --- |
| B1 | Four-crate fixture separates trait, implementation, generic type argument, and caller; works using prebuilt artifacts |
| B2 | Slot order survives importer ID remapping and independent method-map iteration order |
| B3 | DCE preserves metadata-only referenced methods, defaults, destructors, factories, and erased entries |
| B4 | Malformed slot signatures, missing evidence, unbound runtime parameters, and incompatible artifacts fail production validators |
| B5 | Concrete MIR still rejects unresolved generic/inference types; erased MIR admits only bound descriptor-backed parameters |
| B6 | LLVM verification and behavior agree at unoptimized and optimized settings |
| B7 | Metadata interning supports recursion and concurrent construction without deadlock, dangling evidence, or duplicate semantic identities |
| B8 | No-stdlib borrowed-object program works without compiler-owned stdlib discovery |

## 15. Integration map and validation strategy

The following is a dependency map for implementation, not a reduced release scope:

1. **Type/evidence foundations:** semantic and interned types, kind/sizedness rules, object signatures, existential identities, symbolic borrow contracts.
2. **Language and selection:** parser/AST, type resolution, constraints, object coercions, member authority, openings/packaging, supertrait closure.
3. **Runtime body production:** erased ABI, dictionary/descriptor factories, HKT layouts, evidence-bound body validation, metadata reachability.
4. **MIR and backend:** explicit dynamic operations, backend contract, loan/move/drop handling, LLVM lowering, production agreement checks.
5. **Owners and protocols:** allocation layout, `Box`, `Arc`, custom-owner validation, consuming receivers, callable integration.
6. **Artifacts and tooling:** persisted schemas/effects/entries, ID remapping/link records, grammar/formatter/LSP/docs integration.
7. **Whole-feature acceptance:** all section 14 cases, with local and external implementations and optimization-independent behavior.

Artifacts, borrowing, and ownership contracts must be designed from the beginning; this ordering does not permit postponing their semantics until after codegen.

Implementation can begin with the shared type/layout and evidence foundations. Before each new representation becomes a production or artifact boundary, its implementation design must specify its canonical identity, invariants, owning phase, serialization/remapping rules, affected callers, and rejection tests. The language specification does not substitute for those concrete contracts.

In particular, erased callable ABI records, owner protocol signatures, and symbolic borrow/effect summaries must be made precise before their corresponding lowering is implemented. Preserve the proposed surface semantics while that work proceeds; any change to `open`/`pack`, existential typing, or ownership behavior requires an explicit specification revision rather than an implementation-driven shortcut.

The first preparatory refactor replaces the independent inference/selection sizedness implementations with `TypeLayout::sizedness` and explicit `Sizedness` states. Generic evidence remains caller-owned; selection also propagates inherited sizedness evidence through aggregate members. This does not introduce object types, unsized nominal tails, erased execution, or a new artifact format. A sized classification is not permission to bypass concrete-MIR runtime-type validation. Nominal declarations still follow the existing sized-layout rules until their metadata-aware layout contract is implemented.

Primary source touchpoints include:

- `lib/src/ast/`, `lexer/`, `parser/items/parse_type.rs`, adjacent parser tests, `fmt/`, and `macro_expansion/`.
- `lib/src/type_lowering.rs`, `types/mod.rs`, `type_context/`, and `type_services/`.
- `lib/src/collect/`, `lower/`, `infer/`, `selection/`, `hir/`, and `language_items.rs`.
- `lib/src/mono/`, `dce.rs`, `mir/builder/`, `mir/borrowck/`, `mir/agreement.rs`, and `mir/backend_contract.rs`.
- `lib/src/codegen/types.rs`, `codegen/mir_llvm/`, runtime helper declarations, and intrinsic lowering.
- `lib/src/products.rs`, `products/type_table.rs`, `crate_artifact/`, `crate_system/`, and `rock-shared/src/sysroot.rs`.
- `stdlib/alloc.rk`, `box_type.rk`, `arc.rk`, `drop.rk`, `mem.rk`, `callable.rk`, and `thread_safety.rk`, plus containers using generic element layouts.
- `tree-sitter-rock/`, `rock-lsp/`, language reference, and integration tests in `lib/tests/integration.rs`.

For implementation verification, run the smallest relevant parser/type/selection/MIR/artifact test groups first, serialized. Add end-to-end compile/run and compile-fail tests for the acceptance matrix. Then run `cargo test -p rock-lib`, `cargo test --workspace`, `cargo fmt --all --check`, and `cargo clippy --workspace --all-targets`; run tree-sitter generation/tests from its own directory when grammar changes. Save longer test logs once rather than rerunning for output inspection.

The new syntax examples in this specification are future acceptance fixtures, not executable verification of the current checkout. Documentation-only review validates source references, internal consistency, and coverage; passing existing tests cannot prove the proposed feature is implemented.

## 16. Completion boundary and remaining implementation choices

The normative semantics are fixed by this proposal: no `dyn`, explicit storage, complete ownership and supertrait behavior, erased generics including existing HKT capabilities, preserved `Self` relationships, and sound cross-crate borrowing.

Implementation choices that do not alter those semantics include exact Rust record names, physical slot/header packing, descriptor cache data structures, adapter sharing, scratch-storage strategy, and proven specialization/devirtualization optimizations.

The feature must not be declared complete by replacing runtime generics with closed-world enumeration, banning all `Self` methods, omitting consuming receivers, weakening borrow summaries, requiring local-only implementations, or making owners compiler-known by name. Any intentional semantic change to this proposal must be recorded as a specification revision before implementation relies on it.

## 17. Development progress and continuation

This section is the durable development handoff. Update it after each coherent implementation change and its verification; record incomplete work explicitly. A new conversation should read sections 1.2, 15, and 17 before changing code, inspect the working tree, and continue from the next boundary below. Completion here refers to the named foundation only, not the entire feature or all acceptance criteria.

### 17.1 Implementation ledger

| ID | Status | Delivered behavior and code authority |
| --- | --- | --- |
| F01 | Complete, verified | Shared `TypeLayout::sizedness` / `Sizedness` in `lib/src/type_services/layout.rs`; inference and selection migrated; duplicated classifiers removed; generic supertrait sizedness propagates through tuples/arrays; concrete MIR rejection remains strict |
| F02 | Complete, verified | Shared `traits::evidence::implied_trait_bounds` replaces both supertrait traversals; inference, method/callable selection, and static-member lookup use it. Bound-method ambiguity now includes trait arguments rather than silently merging distinct instantiations of one member |
| F03 | Integrated; focused verification recorded below | Canonical object types, scoped `ObjectSelf`, declaration-aware admission, object inference/coercions, associated bindings, and portable type/identity transport |
| F04 | Integrated; focused verification recorded below | Bare-trait source types, borrowed virtual dispatch, defaults, mutation, borrowed-`Self` results, supertrait upcasts, producer-owned vtable schemas, MIR contracts, and allocation-free borrowed construction |
| F05 | Partial; callback execution verified | Runtime-erased generic identity, helper/result-only forwarding, bounded dictionaries, and shared/mutable/consuming callbacks, including later-defined closures and non-Copy results across artifacts; borrowed callback origin transport and broader erased-body/HKT coverage remain unfinished |
| F06 | Focused consuming/owner/cancellation execution verified; broader integration in progress | Canonical split/rebuild/unique release, Box/Arc origins and destruction, concrete/erased consuming calls and bound method values, unchanged external producers at O0/O3, and explicit/forgotten Send gates pass; partial-constructor/generic-State inference and unpublished allocation cancellation pass focused gates, while generic allocator execution and destination construction remain unfinished |
| F07 | Borrowed scoped execution and bounded Type-kind package milestone verified; broader scope incomplete | Rigid `WitnessId` / `Type::Witness`, scoped Open/member/reclose authority and source diagnostics pass focused gates. Sized Type-kind package construction/open/repack, retained borrowing, whole-owner cleanup and source-free artifacts pass19/19 at the current boundary, including O0/O3 and unchanged providers. Constructor-kind packages, owning object openings, destination construction, broader erased-body/HKT execution and checked equality remain unfinished |

The whole feature is **not complete**. Borrowed scoped opening and bounded allocation-free sized Type-kind packages have source/artifact execution acceptance; constructor-kind packages, owning object openings, destination construction and checked equality do not. Current artifact format is **55** in both crates; recovered/scoped-witness/cancellation snapshots used **52**/**53**/**54** respectively. Proc-macro protocol remains **4**: it transports tokens, not AST/HIR, and these contextual forms do not change the token wire shape. The artifact version bump rejects old layouts; it does not certify the complete feature. Examples remain acceptance targets unless backed by an execution fixture; the older F01–F03 results below describe their historical snapshots, not current whole-feature acceptance.

### 17.2 Verification record

F01 verification (2026-09-29):

- `cargo test -p rock-lib sized --lib`: 42 passed.
- `cargo test -p rock-lib`: 2,350 unit tests, 712 main integration tests, and 1 additional parser integration test passed; 1 documentation test ignored.
- `cargo fmt --all --check` and `git diff --check`: passed.
- `cargo clippy -p rock-lib --all-targets`: completed successfully with warnings.
- Full logs: `/tmp/rock-dynamic-dispatch-foundation-tests.log` and `/tmp/rock-dynamic-dispatch-foundation-clippy.log`. These paths are local convenience copies; the commands and recorded outcomes above are the durable record.

F02 verification (2026-09-29):

- `cargo test -p rock-lib traits::evidence::tests --lib`: 6 passed for the initial shared-traversal implementation.
- Regression reproduced with `cargo test -p rock-lib --test integration distinct_supertrait_instantiations_do_not_silently_select_one_method -- --exact`: failed as intended because the compiler incorrectly accepted an ambiguous call. Log: `/tmp/rock-dynamic-dispatch-evidence-red.log`.
- After the fix, `cargo test -p rock-lib --test integration supertrait`: 3 passed, including the new ambiguity regression in both parent orders and the shared-ancestor diamond regression. Log: `/tmp/rock-dynamic-dispatch-evidence-focused.log`.
- `cargo test -p rock-lib`: 2,356 unit tests, 714 main integration tests, and 1 additional parser integration test passed; 1 documentation test ignored. Log: `/tmp/rock-dynamic-dispatch-evidence-tests.log`.
- `cargo fmt --all --check` and `git diff --check`: passed.
- `cargo clippy -p rock-lib --all-targets`: completed successfully with warnings. Log: `/tmp/rock-dynamic-dispatch-evidence-clippy.log`.
- Source review confirmed that the two old `trait_bounds_with_supertraits` implementations and their callers are gone. The `SelectionService::implied_trait_bounds` method is a facade over the shared query, not a second traversal.

F03 partial verification (2026-09-29; not full feature acceptance):

- `cargo check --workspace`: passed, with a warning in `rockup`; log `/tmp/rock-object-workspace-check.log`.
- `cargo test -p rock-lib`: 2,367 unit tests, 714 main integration tests, and 1 additional parser integration test passed; 1 documentation test ignored. Log: `/tmp/rock-object-validated-tests.log`.
- A subsequent exact-instantiation projection regression was added; `cargo test -p rock-lib object --lib` then passed all 39 matching tests. Log: `/tmp/rock-object-final-focused.log`.
- `cargo clippy --workspace --all-targets`: completed successfully with warnings. Log: `/tmp/rock-object-clippy.log`.
- `cargo test --workspace`: passed for the current snapshot, including the additional projection regression and all workspace crates. Log: `/tmp/rock-object-workspace-tests.log`.
- `cargo fmt --all --check` and `git diff --check`: passed.

Recovery and integration verification (2026-09-30):

- Recovered dialogue and final frontend/backend reports from `/home/champii/rock-session-recovery-2026-09-30/`; the current source, not historical patch inputs, remains authoritative.
- Historical later slices recorded in those reports: `object_execution` 5 passes; `object_types` grew from 27 to 29 passes; erased-dispatch focused runs reported 87 passes; `object_artifacts` 2 passes after canonical borrowed-`Self` authority was fixed. These are **historical**, not rerun evidence.
- Integrated all five missing `OwnedObjectCall` walkers in `analysis.rs` and `products.rs`: traverse owner/arguments, validate and remap metadata types, and transport canonical method/protocol/substitution identities.
- Fresh `cargo check --workspace --all-targets`: passed with warnings; log `/tmp/opencode/rock-dynamic-recovered-check.log`.
- Fresh `cargo test -p rock-lib --test owned_object_calls -- --nocapture`: 2 passed, 8 failed. Shared blocking diagnostic: `type of kind Type is not applicable` on constructor applications in owner protocol signatures. Log `/tmp/opencode/rock-dynamic-owned-calls.log`. Passing rejection fixtures do not validate owner runtime behavior.
- Fixed collection's missing canonical trait-target kind registration and added a constructor/State scoping regression. The subsequent owned-call run passed 3/11 and exposed missing static split authority; `object_owners` passed 4/10, with stdlib signature/contract failures still blocking runtime fixtures. Logs `/tmp/opencode/rock-dynamic-owned-calls-2.log` and `/tmp/opencode/rock-dynamic-owners-2.log`.
- Fresh source suites: `object_execution` 5/5, `object_types` 29/29, and `object_artifacts` 2/2 passed. Logs `/tmp/opencode/rock-dynamic-execution.log`, `/tmp/opencode/rock-dynamic-types.log`, `/tmp/opencode/rock-dynamic-artifacts.log`.
- Fresh backend/borrow suites: `object_adapter_tests --lib` 7/7, `object_tests --lib` 12/12, and `mir::borrowck --lib` 70/70 passed. Logs `/tmp/opencode/rock-dynamic-adapters.log`, `/tmp/opencode/rock-dynamic-raw-objects.log`, `/tmp/opencode/rock-dynamic-borrowck.log`.
- Product traversal regression passed; broader `products::tests` exposed a producer ABI identity bug: declaration member layouts were remapped through ambiguous source IDs instead of using already-reserved interface identities. Fixed that path without weakening strict object evidence remapping; subsequent product tests passed 80/80. Logs `/tmp/opencode/rock-dynamic-walkers-2.log`, `/tmp/opencode/rock-dynamic-products-2.log`.
- A full library run passed 2,398 and failed 25 (log `/tmp/opencode/rock-dynamic-unit.log`); it exposed additional frontend regressions and stale ABI/observer fixtures. Migrated artifact fixtures to explicit producer member layouts and consistent nominal arity, and updated the identity audit to cover the shared whole-backend runtime observer. Subsequent loader tests passed 123/123 and identity audits 47/47; logs `/tmp/opencode/rock-dynamic-loader.log`, `/tmp/opencode/rock-dynamic-audit.log`. The full library suite has not yet been rerun after those fixes.
- Callback backend integration now validates canonical argument packs/results, mutable receiver authority and live loans, once-call move state, dictionary layouts, and borrow-free return obligations. `codegen::erased_tests --lib` passed 4/4; source `erased_objects` passed 4 and failed 7 before the raw-drop fixture marker fix, with callback method substitutions still incomplete. Logs `/tmp/opencode/rock-dynamic-erased-unit.log`, `/tmp/opencode/rock-dynamic-erased-callbacks.log`. Borrowed callback results remain rejected pending symbolic origin transport, not supported by these passes.
- Subsequent callback fixes reconcile result-only generic arguments with the caller result, preserve dictionary evidence, and use the canonical payload-drop marker in fixtures. `cargo test -p rock-lib --test erased_objects` passed 11/11, including later-defined closures across artifacts and consuming non-Copy callback results with exactly-once destruction. Log `/tmp/opencode/rock-dynamic-erased-callbacks-3.log`. This does not cover borrowed callback results.
- Owner metadata now goes through the canonical `record_type` collector, including explicit static owned-call/coercion evidence locations. Constructor-kind retention, inherited `?Sized`, canonical split/release selection, deferred coercion inference, and contextual diagnostics were repaired without weakening validation. The latest executed `owned_object_calls` run passed 15/23, with runtime positives still blocked by a false local return origin in canonical `split`; the contemporaneous `object_owners` run passed 5/10, with stdlib cases blocked by static conversion inference. Logs `/tmp/opencode/rock-dynamic-owned-calls-5.log`, `/tmp/opencode/rock-dynamic-owners-3.log`.
- Generic consuming dispatch is integrated with distinct concrete/erased slots, type arguments and dictionaries, direct caller-result storage, and normal-return release; raw consuming selectors remain rejected. The first `cargo test -p rock-lib --test owned_generic_objects` run passed 0/4: all four reached strict MIR agreement and failed with `InvalidRuntimeType` object-contract evidence. Log `/tmp/opencode/rock-dynamic-owned-generic.log`; this is not runtime verification.
- Static conversion inference fixes and two canonical renamed-trait regressions are applied. Their first verification attempt did not execute tests: a concurrently added borrow-origin regression lacked imports and blocked the unit-test build. Log `/tmp/opencode/rock-dynamic-static-conversion.log`; the successful retry is recorded below.
- Investigated the proposed structural-Copy owner concern: `TypeFacts::is_copy` and `is_copy_id` reject every nominal struct/enum, independently of fields or destructor presence; MIR move selection uses that authority. No ownership semantic change is needed. A pointer-argument nominal regression was added; verification remains pending.
- Retried `cargo test -p rock-lib --lib lower::paths::tests::static_trait_`: 2/2 passed; log `/tmp/opencode/rock-dynamic-static-conversion-2.log`. The subsequent `object_owners` run still passed 5/10, but stdlib compilation now passes frontend conversion and fails borrow checking of existing raw fd reinterpretations in `stdlib/fs.rk` and `stdlib/net.rk`. Log `/tmp/opencode/rock-dynamic-owners-4.log`.
- `owned_object_calls` retry did not execute tests: active scoped-witness integration introduced new exhaustive/API compilation requirements. Log `/tmp/opencode/rock-dynamic-owned-calls-6.log`. Library/workspace test/build attempts are paused until that representation is coherently integrated; prior passing binaries do not verify this source state.
- Added an unverified four-crate artifact fixture separating trait, implementation, argument type, and caller, removing all producer sources before caller compilation, checking unchanged artifacts/objects at optimization levels 0 and 3, and rejecting a producer erased slot missing its bound dictionary. Added unverified object-versus-generic hover/bound coverage and a multi-member ordinal regression that reverses imported member IDs and rejects sorting published slots by those IDs. The external tree-sitter grammar repository is not present in this checkout, so no grammar changes or verification are claimed.
- Added unverified thread-transfer owner fixtures: an explicit `Send` guarantee must permit moving/consuming a borrow-free boxed object on a thread; an erased owner without that guarantee must not recover it from its original concrete type. The owner suite's stdlib bootstrap now caches its `Result` so one compilation failure does not trigger five repeated expensive compilations; every dependent fixture still fails with the original diagnostics.
- Scoped-binder transport now checks witness owner IDs and remaps them with their owning body; an unverified traversal regression covers reservation/import identity changes while preserving local binder/value IDs and opened-member dispatch. LSP traversal sees opening sources/bodies/locals, and witness hover derives visible bounds from the canonical opening rather than an inferred concrete implementation. Added unverified ordinary-function/member/field execution regressions for the contextual words `open`, `pack`, `exists`, and `same_type`.
- Producer ABI shape validation now rejects body-local witnesses in static object signatures, slot parameters/results, trait arguments, and erased concrete leaves; a local opening is not a binder for a global producer schema. An unverified corruption regression covers those static positions. Erased schema trait arguments use the same unbound-type rejection rule as ordinary slots.
- Independent shared-crate gate after the format-53 bump: `cargo test -p rock-shared` passed 14 unit tests and 1 documentation test; log `/tmp/opencode/rock-dynamic-shared-format-53.log`. This does not compile or validate the in-progress compiler representation.
- `cargo test -p rockup` passed 48/49; its relocation fixture invoked an existing stale `rockc` binary, which rejects the current stdlib's `?Sized` and owner markers. Log `/tmp/opencode/rock-dynamic-rockup-format-53.log`. Rebuild the current `rockc` once compiler integration is coherent before rerunning packaging/workspace gates; do not relax package validation. The main integration suite now also caches failed stdlib bootstrap diagnostics rather than recompiling the same failed producer for each dependent test.
- October 2 restart verification: with local LLVM 18.1.8, `cargo check -p rock-lib --tests` reached compiler code and failed on interrupted Witness/Open integration: missing parser `seek` import, `HirOpenOwner` equality support, borrowed-name local binding, Witness cases in inference/type-context/visitors, and two vector-versus-set fields in a backend fixture. Log `/tmp/opencode/rock-dynamic-oct02-check-local-llvm.log`. A duplicate Open arm in the analysis child walker was also removed. No tests executed in this check; owner runtime repairs remain unverified.
- The next `cargo check -p rock-lib --tests` confirmed those frontend errors were gone, but overlapped the active erased-MIR CFG migration and failed on remaining linear-body consumers and backend fixtures (37 library / 49 library-test compilation errors). Log `/tmp/opencode/rock-dynamic-oct02-check-2.log`. Hold execution gates until the backend representation and its consumers are coherent; these are intermediate compilation diagnostics, not test failures.
- Added an unverified erased-schema corruption regression covering nested body-local witnesses in trait arguments, parameters, results, required layouts, borrow-free obligations, and dictionary subjects/arguments/member signatures; the fixture derives its erased ABI from the production declaration authority. The product binder-remapping fixture now uses the same local ID for its witness and opened value, matching the actual frontend binder contract.
- Added unverified shadowed-opening hover coverage and a four-crate scoped-`Self` dispatch fixture: a producer opens a borrowed object and passes its value to a `&Self` argument; downstream implementations have different layouts and behavior, all producer sources are removed, and callers at optimization levels 0 and 3 must leave producer artifacts/objects unchanged.
- First October 2 execution gate: `cargo test -p rock-lib --lib products::object_abi_tests` built/linked successfully with local LLVM and passed 7/8, including the new nested erased-schema rejection coverage. Its sole failure was a stale hardcoded format-52 assertion; it now checks agreement with `rock-shared`'s artifact-format constant. Log `/tmp/opencode/rock-dynamic-oct02-schema-tests.log`. The retry executed no tests: new backend scope/Open/Reclose/OpenedCall records still needed six producer/validator/codegen/fixture migrations (`/tmp/opencode/rock-dynamic-oct02-schema-tests-2.log`). A stable backend gate was requested before further representation expansion. This verifies local toolchain usability, not scoped Open runtime execution.
- After the next interruption, `cargo check -p rock-lib --tests` was down to one production error (opened-call codegen using a nonexistent parameter `abi_ty`) and one test-only error (uniform-layout fixture obtaining normalization from the wrong wrapper). Log `/tmp/opencode/rock-dynamic-oct02-check-3.log`; each diagnostic was sent to its owning agent. No tests executed in this check.
- The third schema-test attempt also executed no tests: connecting scoped-function lowering introduced a diagnostic match-arm return-type mismatch, alongside the uniform-layout fixture error in that snapshot. Log `/tmp/opencode/rock-dynamic-oct02-schema-tests-3.log`. The frontend subsequently repaired the fixture using the resolved program's canonical `normalization_env`; backend received the remaining match-arm diagnostic. The new frontend residual-return/conformance remapping and scoped-owner/default regressions remain unverified.
- Fresh `cargo test -p rock-lib --test owned_generic_objects`: **2/4 passed**. Local consuming generic identity/non-Copy storage/once-only destruction and bounded dictionary/release execution passed; cross-artifact execution exited `2` (the fixture's release-count failure code), and a borrow/reuse rejection fixture instead failed at linking with missing `main`. Log `/tmp/opencode/rock-dynamic-oct02-owned-generic.log`. The rejection assertion now includes its source loop body in failure diagnostics; this does not change rejection semantics.
- Fresh `cargo test -p rock-lib --test owned_object_calls`: **10/23 passed**. Runtime positives often fail linking with missing `main`, and six no-link safety fixtures were incorrectly accepted (once-call reuse, immediate owner moves, hidden payload references, and live-view erasure/upcasting). This is a red safety gate, not completion evidence; missing/generalized body reachability and its inference/MIR authority must be investigated before further feature expansion. Artifact State corruption was correctly rejected by scoped evidence validation; its diagnostic fixture now requires structured Artifact classification and the new exact transport/unique-release evidence message. Log `/tmp/opencode/rock-dynamic-oct02-owned-calls.log`; the changed fixture is not yet rerun.
- Fresh `cargo test -p rock-lib --test object_owners`: **6/12 passed**; stdlib bootstrap succeeded, clearing the historical fs/net raw-fd blocker. Four Box/thread fixtures instead fail imported unrestricted-`?Sized` owner-operation proof, Arc clone/upcast exposes a false live receiver-storage loan, and the zero-size allocator fixture incorrectly reused a moved nominal `Layout`. The fixture now builds a separate release-layout value from primitive fields before consuming its allocation layout; no Copy authority was changed. Log `/tmp/opencode/rock-dynamic-oct02-owners.log`; corrected fixture verification is pending.
- Temporary analysis of the first live-view rejection confirms resolver and accepted `main` share `DefId(0,20)`, canonical name `main`, no parameters/generics, and `I64` return/body types; object calls have finalized canonical authority and substitutions. Missing `main` is downstream of that accepted-header boundary, not frontend generalization. Log `/tmp/opencode/rock-dynamic-oct02-main-authority.log`; temporary fixture prints were removed.
- `cargo test -p rock-lib --lib products::`: **113/113 passed**, including producer ordinal, portable binder identity, format agreement, and nested erased-schema Witness rejection coverage. Log `/tmp/opencode/rock-dynamic-oct02-products.log`.
- The HIR runtime-concreteness check incorrectly treated static owner-constructor authority inside an `OwnedObjectCall` as a runtime value. Its record now uses existing concrete-type-argument/method authority for static metadata while retaining strict runtime object/state/operand checks and the Open barrier. `cargo test -p rock-lib --lib hir::owned_objects::tests::closed_owned_call_keeps_static_constructor_authority_codegen_concrete -- --exact`: **1/1 passed**; log `/tmp/opencode/rock-dynamic-oct02-owner-concrete.log`. Owner execution/safety reruns remain necessary.
- Focused Box proof diagnosis reproduced the failure and captured selected imported `into_parts` function `DefId(1,734)` with a sole synthetic `T0` parameter owned by `DefId(1,699)`, `maybe_unsized=false`, no relaxed/positive bounds, and no owner substitution. Log `/tmp/opencode/rock-dynamic-oct02-box-contract.log`; temporary production logging was removed. Provider declaration/body sequencing is being investigated; the validator remains strict.
- Further diagnostic capture identifies provider member `699` as `stdlib::object_owner::ObjectOwner::into_parts`, not Box. Its signature has relaxed `T`; exported Box method `734` already has foreign `T0` without relaxed bounds before import. Log `/tmp/opencode/rock-dynamic-oct02-box-authority.log` used the precise earlier failing artifact for diagnosis only; temporary logging/artifact bypass was removed. Forward trait skeletons can leave earlier impl headers inferred, and pre-body conformance then injects the signature's foreign binder. The required-signature path now seeds a complete method-owned scheme for genuinely unresolved inferred headers before unification, preserving kind/relaxed/positive-bound/predicate authority; explicit schemes and validators stay unchanged. The new reversed-source-order and narrowed-scheme accepted-HIR regression is unverified; a fresh stdlib producer is required for acceptance.
- After the HIR concreteness repair, `owned_generic_objects` passed **3/4**: both local positives and both live-view/reuse rejection bodies pass, eliminating the missing-main failure in this suite. Only cross-artifact release-count execution remains red. Log `/tmp/opencode/rock-dynamic-oct02-owner-safety-2.log`. Cargo stopped at that failed harness, so `owned_object_calls` did not run in the same command; its separate retry is pending.
- Separate `owned_object_calls` retry passed **19/23**; direct consuming/drop/non-Copy and live-loan/hidden-reference checks now pass. Four consuming method-value cases panic in MIR local lookup, including two rejection fixtures; a panic is not semantic rejection. Log `/tmp/opencode/rock-dynamic-oct02-owned-calls-2.log`. Subsequent backend closure/scope and external-release repairs await fresh verification.
- With a freshly rebuilt stdlib after the complete-scheme repair, `object_owners` passed **11/12**: Box, Arc, alignment/zero-size release, and forgotten-Send rejection pass. Explicit-Send thread execution fails mono because its JoinHandle local is tracked as a split-result tuple. Log `/tmp/opencode/rock-dynamic-oct02-owners-2.log`. Frontend subsequently added complete pre-materialization reservation of match-pattern/For/capture/reference local IDs without guessed types; that new regression and runtime repair remain unverified.
- `hir::owned_objects::tests` first executed no tests due to two untyped LoanId fixture arguments, then its retry passed **3/4**. Forward-owner scheme rebinding in both source orders, opened-owner authority, and closed static-constructor authority passed. The remaining forged-state/release/borrowed-owner test fails its message expectation while actually rejecting the borrowed owner. Logs `/tmp/opencode/rock-dynamic-oct02-owner-authority.log` and `/tmp/opencode/rock-dynamic-oct02-owner-authority-2.log`; investigate the exact assertion without weakening rejection.
- Post-backup continuation: the local-ID reservation gate initially executed no tests because the new production pattern walker lacked its `HirPattern` import. Adding the import yielded **1/1** for `infer::authority::worklist_tests::generated_locals_are_reserved_above_patterns_loops_and_captures`; logs `/tmp/opencode/rock-dynamic-resumed-local-ids.log` and `/tmp/opencode/rock-dynamic-resumed-local-ids-2.log`. The borrowed-owner forged fixture now requires its actual owning-value rejection message; `hir::owned_objects::tests` passed **4/4** without changing production validation, log `/tmp/opencode/rock-dynamic-resumed-hir-owners.log`.
- Post-backup `owned_object_calls` passed **23/23**, including all four formerly panicking consuming method-value cases and immediate-move/once-reuse rejection, log `/tmp/opencode/rock-dynamic-resumed-owned-calls.log`. `owned_generic_objects` passed **4/4**, including downstream generic types and normal-return external storage release without rebuilding the producer, log `/tmp/opencode/rock-dynamic-resumed-owned-generic.log`. These close the earlier focused red gates, not the remaining existential or full-library acceptance scope.
- Post-backup `object_owners` passed **12/12** against a freshly compiled stdlib, including explicit-Send threaded consumption, forgotten-Send and shared-Arc consuming-access rejection, Arc clone/upcast destruction, and aligned zero-sized allocation. Log `/tmp/opencode/rock-dynamic-resumed-owners.log`; runtime 289.41 seconds. This verifies the match-binding/generated-local reservation repair without adding local type annotations or mono type guesses.
- Expanded generic-consuming acceptance now exercises direct calls and bound method values for identity/non-Copy results and bounded dictionary calls across unchanged producer artifacts/objects at optimization levels **0 and 3**, plus live-view, immediate owner-move, and once-call reuse rejection. The first expanded run passed 3/4 because one new negative incorrectly called a shared payload method directly on its custom owner; that source was corrected to use the owner's real `borrow` operation, not accepted as move-rejection evidence. The retry passed **4/4**, logs `/tmp/opencode/rock-dynamic-resumed-generic-method-values.log` and `/tmp/opencode/rock-dynamic-resumed-generic-method-values-2.log`.
- Post-backup `object_artifacts` passed **3/4**: generic late-layout/dictionary dispatch and default/Self-upcast three-artifact execution pass. The new four-crate `scoped_self_argument_dispatch_uses_producer_evidence_without_producer_sources` fails during library production with Internal/toolchain diagnostic `opened call does not match its exact scoped witness signature/access`; its scoped execution is not accepted. Log `/tmp/opencode/rock-dynamic-resumed-object-artifacts.log`. Diagnosis locates this first failure in accepted-HIR existential validation before mono; the frontend owns that signature/access repair. A subsequent backend limitation also needs an internal scoped erased-call record rather than pretending erased entries belong to concrete slots.
- Post-backup `cargo test -p rock-lib --lib` passed **2455/2464**, with nine failures: four accepted-HIR scoped-forgery fixtures cannot build their valid baseline, three analysis/hover fixtures fail (one has invalid comma-separated signature syntax), a scoped backend fixture returns 41 instead of 42, and the nonzero language-item remap fixture incorrectly loads two full protocol providers. Stdlib artifact/bootstrap, products, uniform-layout, identity, and remaining unit gates completed; log `/tmp/opencode/rock-dynamic-resumed-lib-units.log`, runtime 232.20 seconds. This is not full-library acceptance; frontend/backend fixes and a strict nonprovider preload fixture correction await focused reruns.
- The exact scoped artifact retry still failed **0/1**, but improved validation diagnostics show `receiver_ok=false`, `bound_ok=true`, `signature_ok=true`: the shared receiver expression is an adjusted bare Witness while signature/argument types are references to that same witness and the result is I64. Log `/tmp/opencode/rock-dynamic-resumed-scoped-artifact-diagnostic.log`. The receiver access/place lane must be repaired without relaxing bound/signature checks.
- The nonzero language-item remapping regression now preloads a well-formed ordinary/nonprovider interface before the sole full protocol provider, rather than expecting conflicting providers to load. Its exact rerun passed **1/1**, log `/tmp/opencode/rock-dynamic-resumed-provider-remap.log`; production provider validation/merging is unchanged.
- After the format-54 cancellation and internal call migrations, the first scoped-HIR gate executed no tests because the new codegen arms were absent; the next attempt also executed no tests due to sibling helper visibility, a fixture calling a private validator, and two missing registry-fixture imports. Logs `/tmp/opencode/rock-dynamic-resumed-hir-scopes.log` and `/tmp/opencode/rock-dynamic-resumed-hir-scopes-2.log`. Narrow production visibility, whole-program public fixture validation, and imports repaired those compilation blockers; `hir::existential::tests` then passed **7/7**, including naked-receiver, forged mutable access, result erasure, and closure-capture rejection. Log `/tmp/opencode/rock-dynamic-resumed-hir-scopes-3.log`.
- `mono::erased::tests::` passed **4/4**, including nested returns and two returning branches without fabricated join values; log `/tmp/opencode/rock-dynamic-resumed-erased-flow.log`. The initial expanded `codegen::erased_tests::` passed **11/13**: the two red fixtures had an invalid direct-name expectation for an LLVM void indirect call and an erased-call Bool literal instead of a typed initialized place. Repairs retain runtime values and exact drop counts without changing validators; retry pending. Log `/tmp/opencode/rock-dynamic-resumed-erased-backend.log`.
- Whole-workspace Rust formatting initially failed while fixture migrations were in progress. After formatting the migrated Rust files, `cargo fmt --all -- --check` passed; log `/tmp/opencode/rock-dynamic-resumed-fmt-4.log`. This is formatting evidence, not full library/workspace execution acceptance.
- Expanded erased backend retry passed **13/13**, including exact scoped native sum at O0/O3, wrong nominal/descriptor rejection, scope/slot/native ABI corruption, generic reference forwarding/reclosing, parameter-vs-local reborrow origins, mutable capability/loan exclusion, and exactly-once non-Copy transfer/drop across CFG paths. Log `/tmp/opencode/rock-dynamic-resumed-erased-backend-2.log`; this is internal backend execution evidence, not yet four-crate source acceptance.
- The next exact scoped artifact gate still failed **0/1** during producer library construction: non-generic additional `Self` inputs were omitted by the erased-schema classifier. The canonical classifier now inspects declaration-owned Self dependencies; producer schema and portable slot-order validation share it, while the native ABI guard remains strict. The initial new unit fixture used an unresolved trait in an accepted-trait map and blocked a `cancellation` unit command before tests; that fixture phase was corrected explicitly. The classifier regression passed **1/1**, log `/tmp/opencode/rock-dynamic-resumed-self-schema.log`. Source artifact retry then compiled producers but failed **0/1** in downstream compilation: imported `read_same` hit the ordinary Open guard and both concrete views lacked canonical erased implementation selection. Log `/tmp/opencode/rock-dynamic-resumed-scoped-artifact-2.log`; import/native-wrapper reachability and implementation authority remain red.
- Cancellation source admission initially passed **3/4**; a malformed receiver reached an ambiguous higher-kinded receiver diagnostic instead of the required protocol-member error. Collection now rejects marked cancellation receivers at their exact member-name spans before header inference, without weakening assertions. The expanded five-test source retry is pending; log `/tmp/opencode/rock-dynamic-resumed-owner-cancellation.log`. Typed cancellation authority ran no tests in its first attempt due to the unrelated unresolved schema fixture, log `/tmp/opencode/rock-dynamic-resumed-cancellation-authority.log`.
- The new generic allocator/partial-owner fixture initially failed **0/1** with Number-vs-Value inference mismatch, log `/tmp/opencode/rock-dynamic-resumed-partial-owner.log`. Canonical unary constructor-section matching replaced two unary-nominal assumptions; fixed arguments and payload position now come from the marked impl pattern, not a last-argument heuristic. Its new `partial_owner_coercion` suite passed **5/6**: fixed allocator/state authority, first-position payload, and wrong allocator/position/unproven-bounds rejection pass. Contextual generic allocator admission remains red (`one proven constructor implementation with independent State`), log `/tmp/opencode/rock-dynamic-resumed-partial-coercion.log`; fix its actual bounds/provenance authority, not by closing or annotating the generic allocator.

- Cancellation source retry passed **5/5**, including exact member-name spans for shared/mutable/move signatures and defaults, log `/tmp/opencode/rock-dynamic-resumed-owner-cancellation-2.log`. Typed cancellation retry first passed **3/4**: its structural-HIR fixture declared private language items. Making those fixture declarations exported (as required of actual providers) preserved validation and yielded **4/4**, covering format-54 roundtrip/remapping, forged cancellation identities/State, constructor kind/State mutation, and registry merge without unique extraction. Logs `/tmp/opencode/rock-dynamic-resumed-cancellation-authority-2.log` and `/tmp/opencode/rock-dynamic-resumed-cancellation-authority-3.log`; runtime cancellation remains a separate gate.
- Partial-owner retry passed **8/8**, including contextual default/explicit allocator bounds and relaxed/unproven bounds rejection, log `/tmp/opencode/rock-dynamic-resumed-partial-coercion-2.log`. Missing default-Sized evidence is derived only from actual canonical value-kind function parameter declarations after inherited bound merging; relaxed, higher-kinded, and undeclared parameters remain unproven. Its identity/relaxation unit passed **1/1**, log `/tmp/opencode/rock-dynamic-resumed-contextual-sized.log`; the unchanged generic-State partial-owner fixture passed **1/1**, log `/tmp/opencode/rock-dynamic-resumed-partial-owner-2.log`. These are inference/accepted-HIR authority gates, not generic allocator-state execution acceptance.
- Expanded imported-native erased backend passed **14/14**, including separate producer/consumer LLVM modules at O0/O3 and ten forged/missing/duplicate link/source/pass-mode mutations, log `/tmp/opencode/rock-dynamic-resumed-erased-backend-3.log`. Imported native specialization regression passed **1/1**, log `/tmp/opencode/rock-dynamic-resumed-import-specialize.log`. The extended schema fixture failed **0/1** because its external crate lacked canonical product identity, log `/tmp/opencode/rock-dynamic-resumed-self-schema-2.log`; fixture repair is pending. Exact four-crate source retry advanced to consumer MIR agreement but failed **0/1**, reporting four invalid backend contracts without detailed erased errors; log `/tmp/opencode/rock-dynamic-resumed-scoped-artifact-3.log`. Source acceptance remains red despite backend fixture success.

- Permanent MIR agreement diagnostics now retain the existing erased-validator error vector as well as its unchanged invalid-contract count. Exact scoped source retry still failed **0/1** with `MissingCallable` twice for each of two canonical imported native bridge keys (per-function and whole-program checks), log `/tmp/opencode/rock-dynamic-resumed-scoped-artifact-4.log`; the actual production import-link evidence remains under repair, with strict validators intact.
- Fresh format-54 stdlib cancellation runtime gate failed **0/1** after successfully rebuilding stdlib: `Box::allocate` and `Box::cancel` had no matching constructor-owned static implementation, log `/tmp/opencode/rock-dynamic-resumed-cancel-runtime.log`. Fresh Arc cancellation-no-unique gate passed **1/1**, log `/tmp/opencode/rock-dynamic-resumed-cancel-arc-authority.log`; this proves unique consumption rejection, not successful allocation/cancellation execution.
- Existential source/tooling suite passed **12/13**, log `/tmp/opencode/rock-dynamic-resumed-existential-source.log`. Shared identity, default-body remapping, helper forwarding, local closure capture, mutable reborrow, reclose, unused-binder hover, invalid-source spans, and other escape rejection pass. Outward-break escape is rejected only by internal accepted-HIR checks and still lacks the required source-backed diagnostic. Analysis tooling passed **6/6**, including canonical generic-hover IDs and distinct shadowed-opening bounds, log `/tmp/opencode/rock-dynamic-resumed-analysis.log`.
- Phase-correct extended schema fixture with actual consumer/provider product identities passed **1/1**, log `/tmp/opencode/rock-dynamic-resumed-self-schema-3.log`; no production symbol fallback was added. Original accepted scoped-body portable retention passed **1/1**, log `/tmp/opencode/rock-dynamic-resumed-portable-scoped-body.log`, and the child-only cancellation marker parser gate passed **1/1**, log `/tmp/opencode/rock-dynamic-resumed-cancel-parser.log`.
- Fresh format-54 owner preservation passed **13/13** `object_owners`, **4/4** `owned_generic_objects`, and **23/23** `owned_object_calls` in one serialized Cargo invocation, log `/tmp/opencode/rock-dynamic-resumed-format54-owners.log`. Additional source/runtime regressions passed **11/11** `erased_objects`, **2/2** contextual syntax, **5/5** object execution, and **29/29** object types, log `/tmp/opencode/rock-dynamic-resumed-object-regressions.log`; borrowed callback origin transport remains outside these successes. MIR borrow checking passed **74/74**, log `/tmp/opencode/rock-dynamic-resumed-borrowck.log`.
- Constructor static-member lookup now infers proven trait arguments before exact canonical constructor-member selection, reusing the existing selection service. The renamed constructor-static suite initially passed **4/5**: wrong State was correctly rejected with `Type mismatch: I64 vs Token`, but its assertion incorrectly expected lowercase text. Requiring that exact State mismatch yielded **5/5**, logs `/tmp/opencode/rock-dynamic-resumed-constructor-statics.log` and `/tmp/opencode/rock-dynamic-resumed-constructor-statics-2.log`. Fresh stdlib cancellation runtime then passed **1/1**, log `/tmp/opencode/rock-dynamic-resumed-cancel-runtime-2.log`: Box and Arc allocate/cancel unpublished zero-sized and 4096-byte-aligned storage without payload destruction or unique-release fallback.
- Scoped native callable disappearance was caused by DCE omitting exact `NativeCall` instance edges from emitted erased bodies. Its real DCE-to-builder local/imported evidence regression passed **1/1**, log `/tmp/opencode/rock-dynamic-resumed-erased-native-dce.log`; missing records still fail without fabricated callables/links. All artifact tests then passed **3/4**, log `/tmp/opencode/rock-dynamic-resumed-artifacts-5.log`: scoped source compilation/linking and MIR agreement now succeed, but its O0 executable terminates by signal instead of returning 42. A diagnostic retry confirms SIGSEGV (Unix wait status 139), log `/tmp/opencode/rock-dynamic-resumed-scoped-artifact-6.log`, with LLVM, objects, and executable preserved at `/tmp/rock_opened_object_artifacts_233132_0`. Artifact fixtures now preserve failure evidence and report exact process status; safety assertions remain unchanged. GDB was blocked by the tool allowlist, so no debugger execution occurred or configuration was changed. Scoped source runtime remains red.
- Full library unit gate passed **2490/2490**, log `/tmp/opencode/rock-dynamic-resumed-lib-units-2.log`, including both ordinary/generated-default break-ending block `Never` regressions. This snapshot predates the approved argument-span metadata migration and the pending scoped artifact runtime repair; it is not whole-feature completion or workspace integration acceptance.
- Expanded existential source retry passed **14/16**, log `/tmp/opencode/rock-dynamic-resumed-existential-source-2.log`: the original outward-break regression now receives a source-backed witness error and independent/valueless breaks pass. Two new strict whole-value span tests expose lost provenance: Dot lowering replaces the receiver span with the member token, while AST Arguments discards the consumed `!` suffix entirely. Metadata-only `Span` transport on existing `SecondaryExpr::Arguments` is approved to capture actual consumed tokens and compose whole call ranges, keeping member references separate. No new existential/packing/equality variant, runtime shape, token protocol bump, guessed source range, or validator relaxation is approved by that repair; migration and rerun are pending.
- The metadata migration landed as `SecondaryExpr::Arguments(Vec<Argument>, Span)`, capturing actual consumed suffix tokens and composing whole call ranges while exact member references remain separate; same-file token envelopes and cross-file terminal-token provenance never fabricate a source range. The parser suffix regression and whole-call/member-reference analysis regression each passed **1/1**, logs `/tmp/opencode/rock-dynamic-resumed-call-suffix-span.log` and `/tmp/opencode/rock-dynamic-resumed-call-reference-spans.log`. Full existential source retry passed **16/16**, including unchanged strict `value.rebuild!` span assertions and source-backed loop/while/array-for escape rejection, log `/tmp/opencode/rock-dynamic-resumed-existential-source-3.log`. The migration adds no feature variant or persisted compiler-product shape; artifact format remains 54 and token-only protocol remains 4. Broad library/workspace and formatting/lint reruns await a stable backend runtime-repair boundary.
- After backend cancellation, direct inspection of preserved `opening.ll` identified infinite native self-recursion: monomorphization resolved the concrete wrapper's forwarding call to an Instance, but MIR scoped-entry routing recognized only Function targets. The object argument uses its real two-word descriptor correctly; no fat/thin layout repair was needed. MIR now recovers the exact registered empty-substitution Function origin from its existing instance map before selecting its scoped entry. Exact source/artifact retry passed **1/1** at O0/O3, log `/tmp/opencode/rock-dynamic-resumed-scoped-artifact-7.log`; all artifact tests then passed **4/4**, log `/tmp/opencode/rock-dynamic-resumed-artifacts-8.log`, including producer-source removal and byte-for-byte unchanged producer artifacts/objects.
- A new real source → accepted HIR → production crate-aware mono → MIR routing regression passed **1/1**, log `/tmp/opencode/rock-dynamic-resumed-wrapper-routing-4.log`: its wrapper is actually resolved to its own canonical Instance before MIR, MIR invokes the separate erased entry, an ordinary same-signature relay remains a native instance call, and agreement is clean. Earlier fixture attempts failed because local records have no external `declared` body and the non-crate-aware mono helper does not perform the production forwarding-call rewrite; logs `/tmp/opencode/rock-dynamic-resumed-wrapper-routing.log`, `-2.log`, and `-3.log`. Fixture construction now uses actual accepted source identities and the production crate-aware pipeline, without weakening assertions or validators.
- Initial workspace verification stopped at CLI **23/44** before library suites because its explicit build-cache sysroot lacked valid bundled stdlib, log `/tmp/opencode/rock-dynamic-resumed-workspace.log`. Current dev rock/rockc/rockup were rebuilt and fresh format-54 stdlib was packaged using the normal `rockup dev stdlib package` command under `/tmp/opencode/rock-dynamic-format54-sysroot`; logs `/tmp/opencode/rock-dynamic-resumed-dev-binaries.log` and `/tmp/opencode/rock-dynamic-resumed-package-sysroot.log`. No ambient sysroot replacement, compiler change, or validator bypass was used for this environment repair.
- Workspace retry with that explicit `ROCK_SYSROOT` passed CLI **44/44**, library units **2493/2493**, erased source **11/11**, and existential source **16/16**, then stopped at integration **694/714**, log `/tmp/opencode/rock-dynamic-resumed-workspace-2.log`. Three safety negatives unexpectedly compiled: `returning_box_as_ref_from_local_box_is_rejected`, `returning_raw_deref_from_local_box_pointer_is_rejected`, and `test_vec_as_slice_borrow_blocks_later_mutation`. Their body-derived copied-contents versus borrowed-storage provenance is under repair; no negative assertion is weakened. Other failures include raw-pointer borrow conflicts, fold return/control-flow inference, signatureless callbacks, generic conversions/default projections, four mixed-safe/unsafe callable diagnostic expectations, and fixture compatibility. No full workspace acceptance is claimed, and subsequent repairs require new gates.
- Remaining focused source/runtime targets passed in one serialized invocation, log `/tmp/opencode/rock-dynamic-resumed-focused-broad.log`: contextual syntax **2/2**, object execution **5/5**, owners **13/13**, object types **29/29**, owned generics **4/4**, consuming calls **23/23**, cancellation source **5/5**, constructor statics **5/5**, partial-owner inference **8/8**, and struct parser integration **1/1**. This snapshot precedes the new integration-driven provenance/inference repairs.
- Compatibility triage has not changed tests or production semantics: the proc-macro integration mock omits current `OpenBrace`/`CloseBrace` discriminants (its Indent/Eol values therefore decode as other token variants); protocol-4 production encode/decode roundtrips passed in the full unit gate. The zero-sized Box test still expects a failed process although the implemented owner allocator intentionally supports zero-sized values. Four mixed-safe/unsafe callable negatives are rejected already, but earlier than their expected unsafe-call diagnostic. These findings are not blanket permission to rewrite expectations, bump protocols, or loosen type safety; generic conversion/default failures remain unresolved.
- Other-crate workspace verification passed rock **44/44**, LSP **4/4**, shared **20/20**, and rockc **44/44**, then stopped at rockup **48/49**, log `/tmp/opencode/rock-dynamic-resumed-workspace-cli.log`. `tests::dev::test_packaged_stdlib_artifact_remains_valid_after_target_component_relocation` packages fresh stdlib using the adjacent dev compiler; it builds that compiler only when the binary is absent. Unsafe writes through raw pointers returned by shared `raw.ptr!` falsely conflicted in String/Vec/network bodies in that pre-repair binary. The provenance lane also owned these raw-pointer positives, distinguishing origin/lifetime tracking from safe-reference shared/exclusive loans without abandoning either borrowed-view safety or copied-contents independence. This failed gate was not fresh post-repair CLI/release acceptance.
- The inference lane supplied an unverified repair boundary in `/tmp/opencode/rock-fold-inference-repair.md`: source return values use the enclosing source function's type rather than synthesized fold callback transport; callback parameter context probes actual pre-coercion preceding arguments and proven trait-element bounds without committing an open object destination. Three source-analysis regressions retain canonical renamed protocols, nested/native-loop returns, user-lambda return scope, exact inferred reference parameters, and scalar-dereference rejection. The first focused analysis-unit attempt executed **no tests**: two compile errors in the other lane's unfinished provenance files (`TypeView::context()` does not exist; a sibling test helper is private), log `/tmp/opencode/rock-dynamic-resumed-fold-context-units.log`. Those errors were returned to their owning lane; no inference success or new unit inventory acceptance is claimed. Further gates await provenance readiness.
- Both lanes then reported stable. The provenance repair distinguishes internal `ParamBorrowedContents` memory views from genuine `ParamContents` copies, preserves actual owner/payload origins through typed raw dereference and slice construction, and separates raw lifetime carriers/reconstructed views from safe shared/exclusive permissions. It adds no artifact/public type shape or compiler-owned container-name exception. First ownership-unit run passed **4/10**, log `/tmp/opencode/rock-dynamic-resumed-borrowed-contents.log`; six new source fixtures failed before MIR because their marked Drop/index roots were not exported. Adding only the required fixture exports yielded **10/10**, log `/tmp/opencode/rock-dynamic-resumed-borrowed-contents-2.log`, preserving every ownership/escape/access assertion. Analysis-unit retry passed **9/9**, including all three new source-return/callback regressions and six existing analysis baselines, log `/tmp/opencode/rock-dynamic-resumed-fold-context-units-2.log`. Original integration negatives/positives and broader ownership/packaging gates remain separate verification obligations.
- The unchanged targeted integration gate passed **12/12**, log `/tmp/opencode/rock-dynamic-resumed-ownership-fold-integration.log`: the three previously unsound local Box/slice negatives, three raw-pointer positives, three fold return/owned cleanup/renamed-protocol regressions, existing fold residual propagation, signatureless mutable-reference callback, and owned/borrowed collection callbacks. All MIR unit tests then passed **263/263**, log `/tmp/opencode/rock-dynamic-resumed-mir-after-provenance.log`, including copied-contents independence and borrow/dataflow baselines. These focused successes do not certify owner/artifact preservation, current CLI relocation packaging, release builds, full library/workspace, or lint/format gates; those remain subsequent serialized checks.
- Post-provenance owner/source/artifact preservation passed in one serialized invocation, log `/tmp/opencode/rock-dynamic-resumed-ownership-preservation.log`: erased source **11/11**, existential source **16/16**, artifact execution **4/4**, stdlib/custom owners **13/13**, owned generics **4/4**, and consuming calls **23/23**. Protected Arc clone/upcast/drop behavior, strict owner safety negatives, additional hidden-Self source dispatch, producer-source removal, and unchanged producer byte checks remain intact. Current CLI relocation packaging, release builds, and broad/quality checks are still subsequent gates.
- The first post-repair packaging attempt failed **0/1**, log `/tmp/opencode/rock-dynamic-resumed-release-relocation.log`; despite that filename it used the unchanged adjacent dev `rockc`, not a rebuilt release compiler. Direct inspection of `rockup/src/tests/dev.rs::ensure_rockc_binary` established this stale-binary boundary. Current dev rock/rockc/rockup were then explicitly rebuilt, log `/tmp/opencode/rock-dynamic-resumed-post-provenance-binaries.log`, without further production changes. The packaging retry is separate from release-build acceptance; no missing compile rebuild is papered over by validator changes.
- With those rebuilt binaries, complete shipped stdlib packaging and relocated artifact validation passed **1/1**, log `/tmp/opencode/rock-dynamic-resumed-current-cli-relocation.log`. Rust formatting was applied and `cargo fmt --all -- --check` passed, logs `/tmp/opencode/rock-dynamic-resumed-fmt-6.log` and `/tmp/opencode/rock-dynamic-resumed-fmt-check-6.log`. `cargo clippy --workspace --all-targets` passed with warnings (not warnings-as-errors or warning-free acceptance), log `/tmp/opencode/rock-dynamic-resumed-clippy.log`. Full workspace retry is running, log `/tmp/opencode/rock-dynamic-resumed-workspace-3.log`; actual release-build acceptance is still separate and unclaimed.
- The completed workspace retry passed CLI **44/44**, full library units **2506/2506**, erased source **11/11**, and existential source **16/16**, then stopped at integration **705/714**, log `/tmp/opencode/rock-dynamic-resumed-workspace-3.log`. All eleven formerly failing ownership/raw-pointer/fold/callback integrations now pass unchanged. The nine remaining failures are `test_function_like_proc_macro_expands_through_public_macro_api`, `test_stdlib_box_rejects_zero_sized_value`, `test_stdlib_string_from_conversions`, `test_string_operations_advanced`, `test_trait_default_method_substitutes_generic_trait_self_output_projection`, and the four `test_unsafe_{if,match}_{returned,tuple}_callable_safe_then_unsafe_requires_unsafe` cases. The unsafe cases still reject earlier than the asserted diagnostic; no assertion or safety validation was relaxed. Cargo did not reach later targets/crates/docs in that workspace command; they require separate gates, and the workspace is not green.
- Actual `cargo build --release` passed with the preserved static LLVM 18 prefix, log `/tmp/opencode/rock-dynamic-resumed-release-build.log`. This verifies release compilation, not all section 14 acceptance or execution/packaging with release binaries; no feature-completion claim follows from the build.
- Crates skipped after the workspace integration failure were verified separately, log `/tmp/opencode/rock-dynamic-resumed-later-crates.log`: LSP **4/4**, shared **20/20**, rockc **44/44**, rockup **49/49**, release-management integration **7/7**, and shared docs **1/1**. Library doctests completed with **0 passed, 1 ignored**, log `/tmp/opencode/rock-dynamic-resumed-lib-docs.log`; this is not an executed documentation-example acceptance. Remaining library source targets passed, log `/tmp/opencode/rock-dynamic-resumed-remaining-sources.log`: contextual syntax **2/2**, object execution **5/5**, object types **29/29**, cancellation **5/5**, constructor statics **5/5**, partial-owner inference **8/8**, and struct parser **1/1**.
- Actual release `rockup` packaged the complete shipped stdlib into `/tmp/opencode/rock-dynamic-format54-release-sysroot` using the explicitly selected release `rockc`, and release `rockc --validate-artifact` accepted that fresh artifact/object layout, logs `/tmp/opencode/rock-dynamic-resumed-release-package.log` and `/tmp/opencode/rock-dynamic-resumed-release-validate.log`. This is release-binary packaging/validation acceptance, not release-binary relocation execution or completion of the unimplemented existential operations. No current gate or implementation lane remains running.

### 17.3 Current boundary and next work

**Active boundary: approved existential-package implementation.** On October 3 the user questioned the previously unapproved `exists … in …` draft surface, then explicitly authorized implementing it if valuable. Its purpose is preserving one shared hidden `Self` across related values that leave an opening scope; the implementation now proceeds under that approval, not merely because the earlier draft included it. Owner-allocating `pack`, owning opening/destination construction, and checked equality are separate and remain held. The passing October 2 gates below are the pre-package baseline, not acceptance of new package shapes.

**Current package and remaining-test milestone:** `cargo test -p rock-lib --no-fail-fast` now passes all targets, log `/tmp/opencode/rock-remaining-lib-all-2.log`: units **2571/2571**, integration **717/717**, packages **19/19** and all protected source/artifact/owner suites. Positive package execution checks both O0/O3 except the separate O0 allocation-absence inspection. Both source-free artifact fixtures remove producer sources and require unchanged artifact/object bytes: shared reference packages accept a later implementation, and owned packages retain one actual imported generic destructor with status0. Native cleanup also preserves an intentional status7. All witness/layout/bounds/access/escape/double-consume negatives retain source-backed non-Internal rejection. The former nine integration failures are resolved as documented below, including explicit corrections of incompatible fixtures rather than compiler weakening. Other workspace crate tests, formatting, Clippy, current-format55 release compilation and explicit stdlib packaging/validation pass in separate serialized commands. Both lanes and all gates are idle; broader dynamic-dispatch completion is not claimed.

Pre-package baseline: scoped source/artifact runtime recursion is repaired; three formerly unsound borrowed-view negatives and targeted raw/fold/callback regressions pass unchanged (**12/12**), MIR units pass **263/263**, and library units pass **2506/2506**. Arc/owners **13/13**, artifacts **4/4**, erased/existential sources **11/11** and **16/16**, owned generics **4/4**, consuming calls **23/23**, current-dev relocation packaging, release compilation/stdlib packaging/validation, later-crate gates, formatting check, and workspace Clippy pass (warnings remain). Integration is **705/714**; those nine failures remain unresolved and no full workspace acceptance is claimed.

Approved package implementation contract:

- One lexical binder reuses existing de Bruijn `BoundVar` identity, binder-aware visitors/substitution, canonical bounds and structural payload identity; renaming the binder does not change the type. Add distinct typed package construction/opening records rather than fabricating an object-handle source or owner split/rebuild proof.
- Package `pack` evaluates and transfers an existing payload once; it does not allocate owner storage. Infer one unique actual witness from every payload occurrence, prove its complete bounds canonically, and preserve hidden origins. Opening creates a fresh body-owned `WitnessId`, consumes the source package, and restores the payload's shared witness. Independent openings remain distinct.
- Physical representation is a structurally proven uniform inline payload with immutable witness-table and exact payload-descriptor pointers. Uniformity is not permission to substitute a guessed concrete type for layout/drop computation. Original initialized/live payload and destructor responsibility transfer exactly once; repacking forwards actual descriptor/evidence pointers, not addresses of temporary evidence slots.
- Initial executable admission is a sized Type-kind witness, by-value package opening, reference aggregates, and exact whole-payload owner transport/drop/repacking. Constructor-kind/relaxed-unsized binders, unsupported scoped-owner projections/reconstruction, and other unimplemented descriptor forms must receive explicit rejection rather than being advertised as supported. This first milestone does not close G4/HKT or all of F07.
- Persisted structural type/HIR additions require artifact format **55** in both crates when landed. Token-only protocol remains **4**. Fresh providers and importer remapping/validation are required; format-54 results above remain historical baselines, not new-schema acceptance.

Two implementation lanes owned frontend/type/product transport and backend/MIR/mono/LLVM respectively; coordinator owns serialized gates and the source/artifact fixtures in `lib/tests/existential_packages.rs`. Their contract/handoffs are `/tmp/opencode/rock-existential-packages-contract.md`, `rock-existential-packages-frontend.md`, and `rock-existential-packages-backend.md`. Both lanes completed the bounded repairs and are idle. Artifact constants are now **55**, token protocol remains **4**. `bd` is unavailable on this host (`command not found`); no alternative issue tracker was introduced. The documented `tree-sitter-rock/` project is absent from this checkout; no grammar generation/test acceptance is claimed.

Initial package verification ledger (before subsequent repairs):

- `cargo check -p rock-lib` passes (`/tmp/opencode/rock-existential-packages-check-4.log`). The first attempt lacked the LLVM prefix; the restored persistent LLVM installation works without system changes. Coordinator repaired missing imports, recursive AST indirection, diagnostic formatting/conversion, a missing conservative package auto-trait arm, and internal serialization visibility. No concrete-classification or validation invariant was relaxed.
- Focused backend validators **11/11**, parser **3/3**, type/binder services **4/4**, accepted HIR **2/2**, and portable product **2/2** pass, with logs `rock-existential-packages-{validators,parser,types,hir,products}-1.log` under `/tmp/opencode`. These do not establish runtime or whole-feature acceptance.
- Source/artifact acceptance is **5/16**, log `/tmp/opencode/rock-existential-packages-source-1.log`. Allocation-free shared-reference construction/open/call executes with exit42; absent-witness inference, missing bounds, mismatched concrete witnesses and independent-opening negatives reject. Remaining function-signature/owner/local-escape cases panic in `MirBuilder::type_id_for` through the strict canonical-interning barrier; the exact backtrace is `rock-existential-packages-backtrace-1.log`. Mutable-reference change followed by read fails erased `BorrowConflict`. Shared-to-mutable rejection is real but lacks a proper access diagnostic. Backend repairs interning/temporary-loan lifetime; frontend repairs diagnostics. No gate runs during their edits.
- Two new fixture corrections follow actual grammar/admission behavior: exported alias uses `type Pair ...` and `< Pair`, not invalid `< type Pair`; nonuniform-layout declaration rejection has existing `Resolve` classification. Message/source-span/no-Internal assertions remain strict. No pre-existing acceptance assertions were rewritten.

The current **19** fixtures establish the bounded sized Type-kind milestone above, including shared-Self dispatch, borrowing/repacking/reassembly, aliases, metadata coherence, allocation-free packaging, exactly-once owner destruction, strict negatives, source-free artifacts and tail/status preservation. The first source/artifact result predates some additions. Constructor-kind packages and broader callback/HKT/owning-construction/equality scope are not completed by these gates.

Subsequent package integration ledger (before the remaining-test repairs):

- Normalized backend interning and call-duration receiver reborrows repaired the initial panics/conflicts without relaxing stored/unique loan checks. Focused package units **33/33** pass (`rock-existential-packages-focused-2.log`); source/artifact suite improves to **15/17** (`rock-existential-packages-source-2.log`). All tested local reference/mutable/alpha/branch/return/reassembly behaviors and strict negatives pass. Artifact alias import and exact owner destruction still fail at that snapshot.
- Native package cleanup now follows existing initialization/move-sensitive flags and performs whole-payload descriptor destruction, not hidden-field projection. Two additional source→MIR/DCE controls pass. Alias root-import/export changes expose a producer canonical-index omission: focused units **35/37**, alias-specific **1/5** (`rock-existential-packages-focused-3.log`, `rock-existential-packages-aliases-1.log`). Frontend repairs source-origin canonical index propagation; missing metadata remains a strict rejection, not a display-name fallback.
- Exact owner O0 runtime now prints one destructor marker but exits65 instead of0 (`rock-existential-packages-owned-drop-3.log`). Saved LLVM stores the synthetic process-status zero only in a temporary and reads an uninitialized return place. Backend repairs return-value propagation across cleanup while preserving intentional integer status. This is not owner runtime acceptance. Both lanes are active; no further gates run during edits. The added owning-artifact and custom-status fixtures remain unrun.
- Those failures are superseded by the current milestone above: aliases are indexed after installation from the collected resolver's canonical ID/name map, and the generic MIR block producer routes its existing explicit unit tail into the actual return destination before lexical cleanup. No fallback return constant, guessed canonical name or validator weakening was added. A new nested-alias fixture was corrected to use real virtual source modules/imports and the actual crate-qualified canonical path; root/nested/phase/TypeId assertions remain intact. Historical failing logs are preserved, not rerun acceptance.
- Final library preservation (`cargo test -p rock-lib --no-fail-fast`, `rock-existential-packages-lib-all-2.log`): units **2553/2553**, packages **19/19**, erased sources **11/11**, borrowed scoped sources **16/16**, prior source-free artifacts **4/4**, owners **13/13**, owned generics **4/4**, consuming calls **23/23**, types **29/29**, execution **5/5**, contextual syntax **2/2**, cancellation **5/5**, constructor statics **5/5**, partial-owner inference **8/8**, and struct parser **1/1** pass. Doctest result remains **0 passed, 1 ignored**. Integration alone is **705/714**, with the same nine failures listed in the pre-package ledger. The command exits101 for that target; it is not a whole-library acceptance pass.
- The first full-unit sweep was **2551/2553** (`rock-existential-packages-lib-units-1.log`): one authorized format bump left a stale cancellation assertion/name at54; one existing native-bridge assertion exposed unnecessary synthesized direct-handle metadata. The current-version assertion/name is updated to55, with previous-format rejection checks unchanged; direct scoped handles now reuse the canonical thin-reference descriptor before aggregate uniform metadata. The bridge assertion was preserved, and its O0/O3 JIT gate plus format55 cancellation/remapping each pass **1/1** before the final sweep. Formatting check (`rock-existential-packages-fmt-check-1.log`) and workspace all-targets Clippy (`rock-existential-packages-clippy-1.log`) pass; Clippy reports warnings, not a warning-free result.
- Current-format tooling gates pass: shared **14/14** plus docs **1/1** (`rock-existential-packages-shared-1.log`) and LSP **4/4** (`rock-existential-packages-lsp-1.log`). Scoped `git diff --check` passes for `lib`, `rock-shared` and this specification; no VCS mutation occurred. No format55 release compilation, CLI/sysroot packaging, whole-workspace acceptance or constructor-kind existential execution is claimed. No implementation lanes or verification gates remain active; current handoffs/logs and the updated specification are preserved in `/home/champii/rock-session-recovery-2026-10-02/`.

Remaining-test repair ledger (October 3, explicitly requested continuation):

- Concrete trait subjects no longer trigger implementation validation while trait arguments remain inference-pending. Caller propagation and numeric defaulting can settle those arguments; unresolved ambiguity still rejects at quiescence, unsupported resolved arguments retain their source-spanned error, and canonical validation owns terminal Solved/Failed state. Structural propagation must not mark resolved arguments as a proven implementation. Solver controls **18/18** pass (`rock-remaining-solve-2.log`); the two String regressions pass without new type annotations or stdlib-name exceptions.
- Inferred if/match return slots bind after branch joins, not to the first branch. Fixed declared expectations, recursive safety variance, callable captures/kinds and pointer/unique-reference invariance remain intact. Early return/break/continue branches contribute existing `Never`, not their payload type, while payloads constrain their actual owning signature. Type refresh preserves `Never`. Explicit discarded statement/`!->` context retains each original typed value and its cleanup before producing unit; it does not leak into bindings, call operands or ordinary lambda/value-return contexts. Final join controls **14/14**, refresh **4/4**, directional safety **1/1** and unsafe integration **24/24** pass. Logs are `rock-remaining-{joins-4,refresh-1,safety-variance-1,unsafe-integration-1}.log` under `/tmp/opencode`.
- Fixture compatibility is corrected explicitly: the proc-macro host mock now mirrors the actual token enum order rather than decoding shifted discriminants; protocol4 and production validation are unchanged. The obsolete zero-size Box rejection expectation becomes size0/non-null/exactly-once destructor/status acceptance, matching the allocator's existing owner contract. The generic trait default returning arbitrary `T` as unconstrained `Self::Output` was universally invalid: the positive now accepts/returns `Self::Output` and tests I64/Bool implementations; the original identity body is retained as a strict compile-fail counterexample. Four original unsafe-call-site negatives are unchanged. Three added integration fixtures cover both-order unsafe runtime joins, the invalid default and discarded owned branch cleanup.
- Intermediate RED evidence is preserved: the first integration attempt **6/716** exposed a common stdlib divergence-join error, not710 independent regressions (`rock-remaining-integration-1.log`); the first broad retry reached units **2567/2567** and integration **715/716**, exposing the unchanged Arc/Mutex test's discarded Unit/I32 branch join (`rock-remaining-lib-all-1.log`). Exact unchanged Mutex and new exactly-once discarded cleanup tests each pass **1/1** before the final sweep. Neither a blanket unit mismatch exemption nor value-type retagging was used.
- Final all-library acceptance is **2571/2571** units, **717/717** integration, packages **19/19**, erased **11/11**, scoped **16/16**, prior artifacts **4/4**, owners **13/13**, owned generics **4/4**, consuming calls **23/23**, types **29/29**, execution **5/5**, contextual syntax **2/2**, cancellation **5/5**, constructor statics **5/5**, partial-owner inference **8/8** and struct parser **1/1**. The command exits0; docs remain **0 passed, 1 ignored**, not executed-example acceptance. Current dev CLI binaries were explicitly rebuilt (`rock-remaining-cli-build-1.log`) before separate CLI tests to avoid stale adjacent `rockc`.
- Other workspace targets pass in separate serialized commands: CLI **44/44** (`rock-remaining-rock-tests-2.log`), LSP **4/4**, shared **20/20** plus docs **1/1**, rockc **44/44**, rockup **49/49** plus release-management integration **7/7** (`rock-remaining-cli-tests-1.log`). The first CLI run was **23/44**: external Cargo target selection denotes a strict explicit sysroot, which lacked a current valid stdlib package. Rebuilt dev `rockup` packaged the shipped stdlib there using explicit rebuilt `rockc` (`rock-remaining-dev-package-2.log`); the affected CLI-only retry passes without repeating unchanged passing crates or relaxing sysroot validation. The wrong source-directory first packaging invocation is retained as `rock-remaining-dev-package-1.log`, not acceptance.
- `cargo fmt --all -- --check`, scoped tracked `git diff --check` and `cargo clippy --workspace --all-targets` pass (`rock-remaining-{fmt-check-1,diff-check-1,clippy-1}.log`). Clippy still reports warnings; no warnings-as-errors/warning-free claim is made. Actual `cargo build --release` passes at format55 (`rock-remaining-release-build-1.log`), and actual release `rockup` packages stdlib with explicitly selected release `rockc`; release artifact validation passes (`rock-remaining-release-{package-1,validate-1}.log`). Separate release-binary O0/O3 smoke execution passes **4/4** for the shared-reference package/status42 and discarded owned-branch exactly-once cleanup/status0 (`rock-remaining-release-smoke-1.log`). These verify those release execution paths, not the remaining HKT/portable borrowed callback contracts or held owner construction/equality scope. No VCS mutation, README edits or `.sisyphus` access occurred; final evidence and handoffs are backed up in `/home/champii/rock-session-recovery-2026-10-02/`.

Current ownership contract:

- `HirExprKindFor::OwnedObjectCall { owner, args, call }` retains the actual owning operand for move/live-loan checking.
- `HirOwnedObjectCall { method, object, state, into_parts, release }` carries canonical consuming member and same-constructor/same-State protocol proof; accepted HIR and artifact loading validate it.
- `MirOwnedObjectKey`, `MirOwnedObjectPlan`, `MirBackendContract.owned_object_calls`, and `Constant::OwnedObjectCall` carry the split → consuming adapter → release sequence. Release occurs only on normal return; it must not run payload destruction a second time. Ordinary and generic consuming fixtures now pass, including method-value immediate-move/once-only checks, hidden payload loans, and external generic release. Descriptor-backed owning openings and destination initialization/commit/cancellation sequencing remain separate unimplemented obligations; verified standalone cancellation does not implement them.

Outstanding completion scope includes borrowed callback origin summaries, broader erased-body/HKT support, generic allocator-state execution (partial-constructor inference has focused acceptance), owning object opening, destination construction for hidden-`Self` owned results, constructor-kind existential packages, checked equality, broader portable evidence/ABI/tooling review, and the remaining section 14 acceptance gates. Borrowed scoped opening and the bounded allocation-free sized Type-kind package milestone have source/artifact acceptance. Owner-allocating packing, owning object opening/destination sequencing and equality remain held; the verified package milestone does not close that scope.

Construction protocol prerequisite identified during October 2 frontend work: `ObjectOwnerAllocate` proves allocation, while `ObjectOwnerUnique` proves initialized-owner extraction/release. The approved `owner_cancel` member and `OwnerAllocateLanguageItems { trait_id, method_id, cancel_id }` registry are implemented with source/product validation, canonical remapping, and Box/Arc cancellation bodies; shared-owner construction does not require unique extraction. Its verification used artifact format54; current package structural additions bump both constants to55, with token-only protocol4 unchanged. Focused cancellation source, typed registry/structural, and Box/Arc runtime cancellation gates pass at that earlier boundary; destination-construction MIR execution is not implemented by those prerequisites. Structural uniform-layout admission is now wired into package admission, separately from exact descriptor/layout/drop authority and not as proof of runtime acceptance.

The review-only owning Open/destination-pack prerequisite in `/tmp/opencode/rock-existential-frontend.md` was inspected: existing Open bindings transport the same constructor/witness/State without allocating; future construction must reuse canonical selected allocation/commit/cancel targets, exact descriptor layouts, initialized-place/drop authority, and dominating loan/evidence facts. Ordinary call authority alone does not certify reservation lifetime or partial initialization. That proposal alone authorizes no construction/equality implementation. The later October 3 user approval separately authorizes allocation-free existential packages, not owner destination-construction sequencing.

#### Representation and authority notes

Scoped-witness integration contract (representation requirements; acceptance evidence is recorded above):

- `WitnessId { owner: DefId, local: HirLocalId }` identifies one opening in a canonical accepted body; remapping changes the owner identity and preserves the body-local binder identity. It is distinct from inference variables, declaration generics, and `ObjectSelf`.
- `HirExprKindFor::Open { source, binding, body }` retains the evaluated source and scoped body; its binding carries the source type, exact object signature, value-local type, and optional canonical owner split/rebuild evidence. Opened member dispatch carries the witness rather than rediscovering a concrete implementation.
- Ordinary concrete runtime/specialization classification rejects witnesses. Descriptor-backed lowering must bind their runtime evidence explicitly; adding a witness must not create a blanket exemption in concrete MIR validation. Copy classification is false, and hidden-reference classification remains conservative until origin evidence is instantiated.
- Internal `OpenedErasedCall` migration is authorized for scoped calls to canonical producer `erased_slots`, with explicit runtime type arguments/dictionaries and scope-proven self/layout evidence. All producers, validators, and backend consumers must migrate together; no public AST/HIR/Type shape is approved by this internal MIR addition. MIR bodies are not artifact-serialized, so this addition alone does not change the public artifact format.
- Internal `NativeCall` migration is also authorized for fully concrete, non-method-generic implementation bodies: an erased ABI wrapper calls an exact canonical monomorphized instance with validated argument/result/pass-mode and borrowing authority. This reuses existing native field/intrinsic execution without fabricating erased support for those operations. Genuinely generic bodies remain descriptor-backed; transferred values and results must keep exact initialization/drop responsibility. No public signature or artifact version change is approved by this internal addition alone.
- Accepted-HIR and artifact validation must check binder dominance, nested scope identity, nonescape, member evidence, and canonical owner operations. Runtime descriptors do not prove compatible hidden regions or permit checked equality on their own. Portable interfaces cannot expose an unbound local witness; body serialization must transport and revalidate its binder.
- Current format **55** adds existential package type/HIR shapes; format **54** added canonical allocation cancellation member identity, and format **53** introduced scoped-witness type/HIR shapes. Proc-macro protocol stays **4** because its token-only payload is unchanged; AST changes alone do not justify a protocol bump. Internal erased MIR call variants are not serialized and do not independently require a format bump. The previous-format rejection fixture uses the actual immediately preceding artifact format rather than a stale literal.

F01 and F02 are verified; do not reimplement them. `lib/src/traits/evidence.rs` owns instantiated implication traversal. It substitutes trait arguments and the implicit/explicit target by canonical generic IDs, expands each structurally distinct bound once, and preserves distinct instantiated ancestors. Predicates about other subjects do not become receiver evidence. Missing/mismatched declaration headers supply no derived evidence; explicit assumptions remain available.

F03 structural representation decisions already implemented:

- `types/object.rs` defines `ObjectType<T>`, `ObjectAssociatedType<T>`, and `ObjectBinding<T>`. The same records use structural `Type` children or interned `TypeId` children; they do not duplicate semantic authority in separate side tables.
- `TraitBound<T>` defaults to structural `Type`. Principal identity stays explicit; additional guarantees and complete binding records use structural ordered sets. `Type` ordering is semantic structural ordering within a compilation, not a display-string order or a vtable ABI.
- Binding identity contains the instantiated trait reference and associated member ID. Binding records also retain their value in set identity, so substitution that merges two keys cannot silently discard contradictory values. Normalization rejects the conflict and removes a redundant explicit principal guarantee.
- Shared visitors/folders traverse trait arguments, guarantee arguments, associated-owner arguments, and binding values. ID remapping reconstructs ordered collections after changing identities. Object projections resolve only an exact instantiated member binding and do not rediscover a concrete impl.
- Portable object rows use product definition/type IDs; duplicate guarantees/bindings, contradictory bindings, invalid child IDs, and evidence/argument resource-limit violations are checked. Artifact trait metadata now uses `ProductTraitTypeHeader` (target kind, argument kinds, associated-member kinds) rather than arity-only data.
- Declaration-aware object admission lives in `traits/objects.rs`; `ObjectSelf` is a scoped hidden witness, not the unsized object type and not a sentinel definition ID. Object-specific inference and coercion evidence now flow through accepted HIR. This witness does not implement user-visible existential opening.
- Ordinary MIR retains its unresolved-generic/inference barrier. Object handles now have explicit metadata contracts and LLVM support; erased runtime parameters have a separate validated representation. Unsupported layouts and missing evidence must still fail instead of reaching placeholder LLVM types.

The implication query requires declaration graph validation and does not normalize type arguments itself. Collection and artifact loading still own cycle rejection (`types::validate_supertrait_graph`). Root/predicate discovery order is deterministic for ordered inputs and independent of display names and hash-map iteration, but is **not** canonical object-signature order or a vtable ABI. Product object ABI records preserve producer-declared member order across imported ID remapping.

F02 exposed and fixes a prerequisite selection bug: `(trait ID, member ID)` alone is not enough to identify a bound-method candidate. `HirSelectedTraitMember` now preserves the instantiated trait arguments during ambiguity checking. Distinct `Tag I64` and `Tag Bool` ancestors cannot silently select one `tag` method; a diamond reaching the same `Base I64` ancestor still supplies one candidate. The new integration regressions exercise both behaviors using existing Rock syntax.

Object-signature canonicalization must keep principal trait, additional guarantees, instantiated associated-member bindings, equality/hashing, kind validation, and complete nested ID remapping consistent. An associated binding identifies its instantiated trait view as well as the associated member; a bare member name or trait ID cannot distinguish all inherited instantiations. Canonicalization must not use formatted/debug type strings or the incidental traversal order of F02.

Future type additions must migrate `Type`, interned `Ty`, shared visitors/folders, normalization/display, inference, portable `ProductTypeRow` encoding/decoding, and artifact validation/remapping together. Persisted representation changes require a coordinated artifact-version bump in `lib/src/products.rs` and `rock-shared/src/sysroot.rs`. Keep unsupported runtime values rejected by the MIR barrier; do not introduce LLVM placeholders or advertise parser-only support.

Erased callable ABI records and split/rebuild/unique-release owner signatures now exist. Allocation-free existential-package `pack` syntax and bounded execution are approved, implemented and verified above. Owner-allocating destination construction remains a held proposal; complete symbolic borrow/effect transport, HKT/runtime aggregate coverage, constructor-kind packages, broader tooling and the rest of section 14 remain completion obligations.

### 17.4 Working-tree and environment notes

- On 2026-10-02, the resumed checkout's source baseline was commit `37433de` (`Intermediaate work`), with `docs/specs/` untracked; this supersedes the earlier all-uncommitted source state. Inspect current files and changes before continuing. The assistant has not staged, committed, or pushed; do not do so without an explicit user request.
- `README.md` contained a pre-existing user modification when this work began; it is outside this feature's edits.
- `bd` was unavailable (`command not found`) during F01. This user-requested progress section must remain sufficient to resume without an external issue database or conversation memory.
- Preserve the full completion scope in section 14. A successful foundation refactor is not a declaration that dynamic dispatch is available.
- After the October 2 restart, lean-ctx MCP is available in **opencode2**; no MCP configuration changes were needed. This Arch host had no LLVM 18 installation, so the initial `cargo check -p rock-lib --tests` stopped in `llvm-sys` before checking compiler code. LLVM 18.1.8 development/static archives and Polly were subsequently extracted outside the repository under `/tmp/opencode/llvm18-local`; use `LLVM_SYS_180_PREFIX=/tmp/opencode/llvm18-local/usr/lib/llvm-18` for this environment. No system packages or compiler linking requirements were changed. The local prefix reaches compiler checking; the failed integration check is recorded in section 17.2.
- Before the user's reboot, task logs/handoffs/downloads, extracted LLVM, and 64 surviving fixture directories were preserved under `/home/champii/rock-session-recovery-2026-10-02/`. Read `CONTINUE-HERE.md` there; use the persistent LLVM prefix `/home/champii/rock-session-recovery-2026-10-02/llvm18-local/usr/lib/llvm-18` after reboot. This replaces dependence on disposable `/tmp` paths, not the authoritative checkout or full acceptance ledger.

## Appendix A. Worked language examples

Examples A.1–A.5 illustrate the full proposed behavior and require their respective acceptance gates; they are not blanket current-feature claims. A.6 illustrates the verified bounded package milestone. Standard owners are imported explicitly; other user-defined names are declared within each example.

### A.1 One generic slot, several argument types

```rock
trait Identity
    @identity: T -> T

struct Echo

impl Identity for Echo
    @identity = value -> value

exercise: &Identity -> ()
exercise = object !->
    number: I64 = object.identity 42
    flag: Bool = object.identity true
    pair: (I64, Bool) = object.identity (number, flag)

main = !->
    echo = Echo
    exercise &echo
```

All three calls select the same member slot. They supply different descriptors and result layouts. The method moves its argument to its result; it does not acquire a `Copy` guarantee merely because these particular examples are copyable. A cross-crate version must additionally use an owned, non-copyable caller-defined type.

### A.2 Owned upcasting and consuming dispatch

```rock
> stdlib::box_type::Box

trait Base
    @read: I64
    ~@finish: I64

trait Derived where Self: Base
    @extra: I64

struct Item
    value: I64

impl Base for Item
    @read = -> self.value
    ~@finish = -> self.value

impl Derived for Item
    @extra = -> self.value

consume: Box Base -> I64
consume = owner -> owner.finish!

main = !->
    item = Item
        value: 9
    derived: Box Derived = Box::new item
    base: Box Base = derived
    before = base.read!
    after = consume base
```

The upcast preserves the allocation. The final call consumes `base`, transfers its payload to `finish`, and releases the original owner storage without a second payload drop. Reading `base` afterward must fail.

### A.3 Explicit allocation for a `Self` result

```rock
> stdlib::box_type::Box

trait Rebuild
    @rebuild: Self

struct Token
    value: I64

impl Rebuild for Token
    @rebuild = -> Token
        value: self.value

rebuild_owned: &Rebuild -> Box Rebuild
rebuild_owned = object -> pack (Box Rebuild) (object.rebuild!)

main = !->
    token = Token
        value: 4
    rebuilt = rebuild_owned &token
```

The allocation is requested by `pack`. The virtual call constructs the hidden result into the owner's destination. Omitting `pack` does not cause an implicit allocation or make an unbound hidden type a legal return type.

### A.4 Preserving a shared witness across two owners

```rock
> stdlib::box_type::Box

trait Rebuild
    @rebuild: Self
    @accept: &Self -> ()

struct Token
    value: I64

impl Rebuild for Token
    @rebuild = -> Token
        value: self.value
    @accept = other !-> ()

make_pair: &Rebuild -> (exists T where T: Rebuild in (Box T, Box T))
make_pair = object ->
    open object as T, value
        first: Box T = Box::new (value.rebuild!)
        second: Box T = Box::new (value.rebuild!)
        pack (exists U where U: Rebuild in (Box U, Box U)) ((first, second))

main = !->
    token = Token
        value: 5
    pair = make_pair &token
    open pair as T, owners
        owners.0.accept (owners.1.as_ref!)
```

The existential package keeps both owners tied to one witness. Opening it restores that relation, so `accept` needs no runtime type check. Replacing the tuple with two independently erased `Box Rebuild` values would lose this static equality evidence.

### A.5 Checked equality for independently formed views

```rock
trait Accept
    @accept: &Self -> I64

struct Number
    value: I64

impl Accept for Number
    @accept = other -> other.value

main = !->
    first = Number
        value: 10
    second = Number
        value: 20
    left: &Accept = &first
    right: &Accept = &second
    result = open left, right as T, a, b if same_type
        a.accept b
    else
        0
```

The success branch produces `20`. The local formation sites establish borrow-free payload types, so runtime concrete identity can safely refine their hidden witnesses. A version using different concrete implementations takes the failure branch; a version lacking compatible hidden-region evidence must be rejected rather than treating lifetime-erased identity as sufficient.

### A.6 Allocation-free shared-reference package (implemented)

```rock
trait Accept
    @accept: &Self -> I64

struct Number
    < value: I64

impl Accept for Number
    @accept = other -> ~I64Add self.value, other.value

type Pair = exists T where T: Accept in (&T, &T)

sum_pair: Pair -> I64
sum_pair = package ->
    open package as Hidden, pair
        pair.0.accept pair.1

main = ->
    left = Number
        value: 20
    right = Number
        value: 22
    package = pack (Pair) ((&left, &right))
    sum_pair package
```

This program intentionally returns process status42. `pack` transfers the reference tuple into inline package storage without allocating an owner. `sum_pair` consumes the package and opens one fresh hidden witness shared by both references, making the `&Self` argument valid without checked equality. The references still borrow `left` and `right`; packaging them does not extend either lifetime. This behavior is covered by `reference_package_restores_one_self_for_both_payload_positions` and the source-free reference-package fixture in `lib/tests/existential_packages.rs`.
