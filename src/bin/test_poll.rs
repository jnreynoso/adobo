#[allow(warnings)]
fn main() {
    let dev: &vello::wgpu::Device = unsafe { std::mem::zeroed() };
    let _ = dev.poll(vello::wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
}
