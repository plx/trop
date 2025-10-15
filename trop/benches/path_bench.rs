use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::path::Path;
use trop::path::{normalize, PathRelationship, PathResolver};

fn bench_normalize(c: &mut Criterion) {
    let mut group = c.benchmark_group("normalize");

    // Benchmark absolute path normalization
    group.bench_function("absolute_path", |b| {
        b.iter(|| normalize::normalize(black_box(Path::new("/absolute/path/to/file"))));
    });

    // Benchmark relative path normalization
    group.bench_function("relative_path", |b| {
        b.iter(|| normalize::normalize(black_box(Path::new("./relative/path"))));
    });

    // Benchmark path with . and .. components
    group.bench_function("with_dots", |b| {
        b.iter(|| normalize::normalize(black_box(Path::new("/a/b/../c/./d"))));
    });

    // Benchmark path with many .. components
    group.bench_function("many_dots", |b| {
        b.iter(|| normalize::normalize(black_box(Path::new("/a/b/c/d/../../e/f"))));
    });

    // Benchmark tilde expansion
    group.bench_function("tilde_expansion", |b| {
        b.iter(|| normalize::normalize(black_box(Path::new("~/project/src"))));
    });

    group.finish();
}

fn bench_normalize_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("normalize_operations");

    // Benchmark tilde expansion only
    group.bench_function("expand_tilde", |b| {
        b.iter(|| normalize::expand_tilde(black_box(Path::new("~/test"))));
    });

    // Benchmark component resolution only
    group.bench_function("resolve_components", |b| {
        b.iter(|| normalize::resolve_components(black_box(Path::new("/a/b/../c/./d"))));
    });

    group.finish();
}

fn bench_relationship(c: &mut Criterion) {
    let mut group = c.benchmark_group("relationship");

    let ancestor = Path::new("/users/test/projects/trop");
    let descendant = Path::new("/users/test/projects/trop/src/path");
    let unrelated1 = Path::new("/users/test/projects/trop/src");
    let unrelated2 = Path::new("/users/test/projects/other");

    // Benchmark ancestor relationship
    group.bench_function("ancestor", |b| {
        b.iter(|| PathRelationship::between(black_box(ancestor), black_box(descendant)));
    });

    // Benchmark descendant relationship
    group.bench_function("descendant", |b| {
        b.iter(|| PathRelationship::between(black_box(descendant), black_box(ancestor)));
    });

    // Benchmark same relationship
    group.bench_function("same", |b| {
        b.iter(|| PathRelationship::between(black_box(ancestor), black_box(ancestor)));
    });

    // Benchmark unrelated relationship
    group.bench_function("unrelated", |b| {
        b.iter(|| PathRelationship::between(black_box(unrelated1), black_box(unrelated2)));
    });

    // Benchmark is_within helper
    group.bench_function("is_within", |b| {
        b.iter(|| PathRelationship::is_within(black_box(descendant), black_box(ancestor)));
    });

    // Benchmark contains helper
    group.bench_function("contains", |b| {
        b.iter(|| PathRelationship::contains(black_box(ancestor), black_box(descendant)));
    });

    group.finish();
}

fn bench_resolver(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolver");

    let resolver = PathResolver::new().with_nonexistent_warning(false);
    let path = Path::new("/tmp/test/path");

    // Benchmark explicit resolution
    group.bench_function("resolve_explicit", |b| {
        b.iter(|| resolver.resolve_explicit(black_box(path)));
    });

    // Benchmark implicit resolution (non-existent path, so no canonicalization)
    group.bench_function("resolve_implicit_nonexistent", |b| {
        b.iter(|| resolver.resolve_implicit(black_box(path)));
    });

    // Benchmark with different path types
    for (name, test_path) in [
        ("absolute", "/absolute/path/to/file"),
        ("relative", "./relative/path"),
        ("with_dots", "/a/b/../c/./d"),
        ("tilde", "~/project"),
    ] {
        group.bench_with_input(
            BenchmarkId::new("resolve_explicit_varied", name),
            &test_path,
            |b, &path_str| {
                b.iter(|| resolver.resolve_explicit(black_box(Path::new(path_str))));
            },
        );
    }

    group.finish();
}

fn bench_resolver_with_canonicalization(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolver_with_canon");

    // Use a path that actually exists to test canonicalization
    let resolver = PathResolver::new().with_nonexistent_warning(false);
    let existing_path = Path::new("/tmp");

    // Benchmark implicit resolution (with canonicalization)
    group.bench_function("resolve_implicit_existing", |b| {
        b.iter(|| resolver.resolve_implicit(black_box(existing_path)));
    });

    // Benchmark forced canonicalization
    group.bench_function("resolve_canonical", |b| {
        b.iter(|| resolver.resolve_canonical(black_box(existing_path)));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_normalize,
    bench_normalize_operations,
    bench_relationship,
    bench_resolver,
    bench_resolver_with_canonicalization
);
criterion_main!(benches);
