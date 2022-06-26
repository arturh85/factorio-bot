/// (windows only) attaches console to parent process so CLI & REPL works without opening
/// a console window on normal start.
///
/// Source: https://www.reddit.com/r/learnrust/comments/jaqfcx/windows_print_to_hidden_console_window/
/// Reason for feature Win32_System_Console
// pub fn attach_console_to_parent_process() {
//     #[cfg(all(not(debug_assertions), target_os = "windows"))]
//     {
//         unsafe {
//             use windows_sys::Win32::System::Console::AttachConsole;
//             use windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS;
//             AttachConsole(ATTACH_PARENT_PROCESS);
//         }
//     }
// }
