use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use openworkers_runtime_jsc::{Runtime, run_event_loop};
use std::time::Duration;

fn bench_stream_manager(c: &mut Criterion) {
    use openworkers_runtime_jsc::StreamManager;
    use std::sync::Arc;

    let mut group = c.benchmark_group("StreamManager");

    // Benchmark stream creation
    group.bench_function("create_stream", |b| {
        let manager = Arc::new(StreamManager::new());
        b.iter(|| manager.create_stream("test-url".to_string()));
    });

    // Benchmark write + read cycle
    group.bench_function("write_read_cycle", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = Arc::new(StreamManager::new());

        b.iter(|| {
            let stream_id = manager.create_stream("test".to_string());
            let manager_clone = manager.clone();

            rt.block_on(async {
                use openworkers_runtime_jsc::StreamChunk;
                let data = bytes::Bytes::from_static(b"Hello, World!");
                manager_clone
                    .write_chunk(stream_id, StreamChunk::Data(data))
                    .await
                    .unwrap();
                manager_clone
                    .write_chunk(stream_id, StreamChunk::Done)
                    .await
                    .unwrap();

                // Read chunks
                loop {
                    match manager_clone.read_chunk(stream_id).await {
                        Ok(StreamChunk::Done) => break,
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
            });
        });
    });

    group.finish();
}

fn bench_response_streaming(c: &mut Criterion) {
    let mut group = c.benchmark_group("Response_Streaming");

    // Different payload sizes
    let sizes = vec![("1KB", 1024), ("10KB", 10 * 1024), ("100KB", 100 * 1024)];

    for (name, size) in sizes {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(
            BenchmarkId::new("create_response", name),
            &size,
            |b, &size| {
                let rt = tokio::runtime::Runtime::new().unwrap();

                b.iter(|| {
                    rt.block_on(async {
                        let (mut runtime, scheduler_rx, callback_tx, stream_manager) =
                            Runtime::new();

                        let event_loop_handle = tokio::spawn(async move {
                            run_event_loop(scheduler_rx, callback_tx, stream_manager).await;
                        });

                        // Create a response with a body of the given size
                        let script = format!(
                            r#"
                        const body = "x".repeat({});
                        const response = new Response(body);
                        response.text();
                        "#,
                            size
                        );

                        let _ = runtime.evaluate(&script);

                        // Process callbacks briefly
                        for _ in 0..5 {
                            runtime.process_callbacks();
                            tokio::time::sleep(Duration::from_micros(100)).await;
                        }

                        drop(runtime);
                        let _ = tokio::time::timeout(Duration::from_millis(100), event_loop_handle)
                            .await;
                    });
                });
            },
        );
    }

    group.finish();
}

fn bench_readable_stream_read(c: &mut Criterion) {
    let mut group = c.benchmark_group("ReadableStream");

    group.bench_function("read_small_chunks", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();

        b.iter(|| {
            rt.block_on(async {
                let (mut runtime, scheduler_rx, callback_tx, stream_manager) = Runtime::new();

                let event_loop_handle = tokio::spawn(async move {
                    run_event_loop(scheduler_rx, callback_tx, stream_manager).await;
                });

                let script = r#"
                    let chunks = [];
                    const stream = new ReadableStream({
                        start(controller) {
                            for (let i = 0; i < 10; i++) {
                                controller.enqueue(new TextEncoder().encode("chunk" + i));
                            }
                            controller.close();
                        }
                    });

                    const reader = stream.getReader();
                    async function readAll() {
                        while (true) {
                            const { done, value } = await reader.read();
                            if (done) break;
                            chunks.push(value);
                        }
                        return chunks.length;
                    }
                    readAll();
                "#;

                let _ = runtime.evaluate(script);

                for _ in 0..20 {
                    runtime.process_callbacks();
                    tokio::time::sleep(Duration::from_micros(100)).await;
                }

                drop(runtime);
                let _ = tokio::time::timeout(Duration::from_millis(100), event_loop_handle).await;
            });
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_stream_manager,
    bench_response_streaming,
    bench_readable_stream_read
);
criterion_main!(benches);
