// napi-rs build hook: wires up the N-API symbol registration and .node
// artifact naming (SPEC-010 NODE-001).
fn main() {
    napi_build::setup();
}
