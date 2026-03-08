use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use gitv_tui::bench_support::{
    build_issue_body_preview_for_bench, issue_body_fixture, markdown_fixture,
    render_markdown_for_bench,
};

fn bench_issue_list_preview(c: &mut Criterion) {
    let mut group = c.benchmark_group("issue_list_preview");
    for repeat in [1_usize, 4, 12, 32] {
        let body = issue_body_fixture(repeat);
        group.throughput(Throughput::Bytes(body.len() as u64));
        for width in [40_usize, 80, 120] {
            group.bench_with_input(
                BenchmarkId::new(format!("repeat_{repeat}"), width),
                &width,
                |b, &width| {
                    b.iter(|| {
                        build_issue_body_preview_for_bench(black_box(&body), black_box(width))
                    });
                },
            );
        }
    }
    group.finish();
}

fn bench_markdown_render(c: &mut Criterion) {
    let mut group = c.benchmark_group("markdown_render");
    for repeat in [1_usize, 2, 6] {
        let markdown = markdown_fixture(repeat);
        group.throughput(Throughput::Bytes(markdown.len() as u64));
        for (width, indent) in [(48_usize, 2_usize), (80, 2), (100, 4)] {
            group.bench_with_input(
                BenchmarkId::new(format!("repeat_{repeat}_indent_{indent}"), width),
                &width,
                |b, &width| {
                    b.iter(|| {
                        render_markdown_for_bench(
                            black_box(&markdown),
                            black_box(width),
                            black_box(indent),
                        )
                    });
                },
            );
        }
    }
    group.finish();
}

criterion_group!(ui_hotspots, bench_issue_list_preview, bench_markdown_render);
criterion_main!(ui_hotspots);
