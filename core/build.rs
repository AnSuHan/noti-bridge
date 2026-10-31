fn main() {
    println!("cargo:rerun-if-changed=../proto/noti_bridge.proto");
    let descriptors = protox::compile(["noti_bridge.proto"], ["../proto"]).expect("proto 컴파일 실패");
    prost_build::Config::new()
        .compile_fds(descriptors)
        .expect("proto 코드 생성 실패");
}
