// Puts icon_songrequestz.ico into the Windows exe (Explorer, taskbar). No-op on other targets.
fn main() {
    embed_resource::compile("songrequestz.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
