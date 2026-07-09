# Rust Ownership — A Complete Guide

## Introduction

Ownership is one of Rust's most distinctive features and the foundation of its memory safety guarantees. Unlike languages with garbage collection or languages requiring manual memory management, Rust uses an ownership system checked at compile time to ensure memory safety without runtime overhead.

## The Three Rules of Ownership

Every variable in Rust has one owner. When the owner goes out of scope, the value is dropped. A value can have at most one owner at a time.

### Rule 1: Each Value Has One Owner

```rust
let s = String::from("hello");
// s is the owner of the String value
```

### Rule 2: Ownership Can Be Transferred

When you assign a variable to another, ownership transfers (not the value itself, but the responsibility):

```rust
let s1 = String::from("hello");
let s2 = s1;  // Ownership transfers from s1 to s2
// s1 is no longer valid; println!(s1) would be a compile error
```

### Rule 3: The Owner Cleans Up

When a value's owner goes out of scope, Rust automatically calls `drop()` to free memory:

```rust
{
    let s = String::from("hello");
}  // s goes out of scope; memory is freed here
```

## Borrowing

To use a value without transferring ownership, you can _borrow_ it by taking a reference:

```rust
let s1 = String::from("hello");
let len = calculate_length(&s1);  // Borrowing a reference to s1
println!("'{}' has length {}", s1, len);  // s1 is still valid
```

### Mutable Borrowing

You can lend a mutable reference to allow the borrower to modify the value:

```rust
let mut s = String::from("hello");
add_world(&mut s);  // Mutable borrow

fn add_world(s: &mut String) {
    s.push_str(" world");
}
```

**Crucial Rule:** You can have either one mutable reference OR many immutable references to a value at the same time. This prevents data races at compile time.

## Move vs. Copy

Small types like integers, floats, and booleans implement the `Copy` trait. These are implicitly copied, not moved:

```rust
let x = 5;
let y = x;  // x is copied, not moved
println!("{}", x);  // x is still valid because integers copy
```

Large types like `String` and `Vec` _move_ ownership by default.

## Conclusion

Rust's ownership system is its secret weapon: it prevents entire classes of bugs (use-after-free, buffer overflows, data races) without needing a garbage collector. Mastering ownership is the key to writing efficient Rust code.
