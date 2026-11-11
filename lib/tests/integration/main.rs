//! Integration tests for the Rock compiler
//!
//! These tests compile example .rk files and verify their output.

#[path = "../support/std_cache_key.rs"]
mod std_cache_key;

#[path = "../support/callable.rs"]
mod callable;

mod support;
use support::{compile_and_run, compile_and_run_without_std, compile_should_fail};

mod array_for_moves;
mod arrays_slices;
mod borrowck;
mod closures;
mod const_generics;
mod const_generics_body_lengths;
mod default_type_arguments;
mod do_notation;
mod drops;
mod enums;
mod files;
mod fold_loops;
mod generics_codegen;
mod hash_map;
mod hkt;
mod index_traits;
mod indexing;
mod integer_arithmetic;
mod io_net;
mod language_basics;
mod macros;
mod method_receivers;
mod modules;
mod operators;
mod option_result;
mod partial_type_annotations;
mod programs;
mod raw_pointers;
mod return_loans;
mod signatureless_inference;
mod std_cache;
mod std_conversions;
mod std_iteration;
mod structs;
mod threads_sync;
mod thread_capture_diagnostics;
mod trait_bounds;
mod trait_dispatch;
mod try_protocol;
mod type_checking;
mod unsafe_callables;
mod utf8_strings;
mod vec;
