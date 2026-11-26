//! M4 Gate：容器 golden、损坏输入、异步加载与 cook 复现。

use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

use mir_asset::{
    AssetError, AssetServer, AssetState, AudioAsset, SceneAsset, decode, encode,
    encode_with_version,
};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

#[test]
fn container_roundtrip_and_corruption_are_diagnosable() {
    let payload = b"miriya asset container".repeat(64);
    let bytes = encode(&payload).unwrap();
    assert_eq!(decode(&bytes).unwrap(), payload);

    let mut truncated = bytes.clone();
    truncated.pop();
    assert!(matches!(decode(&truncated), Err(AssetError::Corrupt(_))));

    let wrong_version = encode_with_version(&payload, 999).unwrap();
    assert!(matches!(
        decode(&wrong_version),
        Err(AssetError::Version { actual: 999, .. })
    ));
}

#[test]
fn cooked_demo_asset_is_byte_reproducible() {
    let workspace = workspace_root();
    let source = workspace.join("assets/fixtures/tone.wav");
    let source_root = workspace.join("assets/fixtures");
    let output_root = workspace.join("target/asset-golden");
    let cooked = mir_build::cook_file(&source, &source_root, &output_root).unwrap();
    let expected = std::fs::read(workspace.join("assets/demo/tone.miasset")).unwrap();
    assert_eq!(std::fs::read(cooked).unwrap(), expected);
}

#[test]
fn demo_asset_loads_asynchronously_into_runtime_cache() {
    let workspace = workspace_root();
    let mut server = AssetServer::new();
    server.set_root(workspace.join("assets"));
    let handle = server.load::<AudioAsset>("demo/tone.miasset").unwrap();

    let mut settled = false;
    for _ in 0..500 {
        server.poll();
        if server
            .state(handle.id())
            .is_some_and(|state| state.is_settled())
        {
            settled = true;
            break;
        }
        sleep(Duration::from_millis(2));
    }
    assert!(settled, "异步资产作业应在预算内完成");
    assert!(matches!(
        server.state(handle.id()),
        Some(AssetState::Ready(_))
    ));
    let audio = server.get(handle).expect("cooked 音频应可读取");
    assert_eq!(audio.sample_rate, 8000);
    assert_eq!(audio.channels, 1);
    assert_eq!(audio.samples.len(), 8000);
}

#[test]
#[ignore = "冷加载基准：设置 MIRIYA_ASSET_BENCH_1GB=1 开启 1 GB 数据集"]
fn cold_load_poll_benchmark() {
    let full_size = std::env::var_os("MIRIYA_ASSET_BENCH_1GB").is_some();
    let asset_count = if full_size { 1024 } else { 64 };
    let payload = vec![7u8; 1_000_000];
    let cooked = encode(&payload).unwrap();

    let root = workspace_root().join("target/cold-load-assets");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    for index in 0..asset_count {
        std::fs::write(root.join(format!("asset-{index}.miasset")), &cooked).unwrap();
    }

    let mut server = AssetServer::new();
    server.set_root(&root);
    let handles: Vec<_> = (0..asset_count)
        .map(|index| {
            server
                .load::<SceneAsset>(format!("asset-{index}.miasset"))
                .unwrap()
        })
        .collect();

    let started = Instant::now();
    let mut max_poll = Duration::ZERO;
    loop {
        let poll_started = Instant::now();
        server.poll();
        let elapsed = poll_started.elapsed();
        if elapsed > max_poll {
            max_poll = elapsed;
        }
        if handles.iter().all(|handle| {
            server
                .state(handle.id())
                .is_some_and(|state| state.is_settled())
        }) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(180),
            "冷加载基准超时"
        );
        sleep(Duration::from_millis(1));
    }

    let max_poll_ms = max_poll.as_secs_f64() * 1000.0;
    println!(
        "冷加载 {asset_count} 个资产（约 {} MB）：总耗时 {:?}，主线程单次 poll 最大 {max_poll_ms:.3} ms",
        asset_count,
        started.elapsed()
    );
    assert!(
        max_poll_ms <= 16.0,
        "主线程单帧阻塞 {max_poll_ms:.3} ms 超出预算"
    );
    let _ = std::fs::remove_dir_all(&root);
}
