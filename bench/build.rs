fn main() {
    cc::Build::new()
        .file("onnx_compat.c")
        .compile("onnx_compat");
}
