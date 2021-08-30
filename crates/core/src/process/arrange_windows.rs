use miette::{DiagnosticResult, IntoDiagnostic};
use std::time::Duration;

/// arrange factorio windows to fill the primary display
pub async fn arrange_windows(client_count: u8) -> DiagnosticResult<()> {
    #[cfg(windows)]
    {
        mod bindings {
            windows::include_bindings!();
        }
        use bindings::Windows::Win32::{
            Foundation::{BOOL, HWND, LPARAM, PWSTR},
            UI::WindowsAndMessaging::{
                EnumWindows, GetSystemMetrics, GetWindowTextW, MoveWindow, SM_CXMAXIMIZED,
                SM_CYMAXIMIZED,
            },
        };
        async_std::task::sleep(Duration::from_secs(client_count as u64)).await; // wait for window to be visible, hopefully
        static mut HWNDS: Vec<HWND> = Vec::new();
        extern "system" fn enum_window(window: HWND, _: LPARAM) -> BOOL {
            unsafe {
                let mut text: [u16; 512] = [0; 512];
                let len = GetWindowTextW(window, PWSTR(text.as_mut_ptr()), text.len() as i32);
                let text = String::from_utf16_lossy(&text[..len as usize]);
                if !text.is_empty() && text.contains("Factorio ") && !text.contains("Factorio Bot")
                {
                    HWNDS.push(window);
                }
                BOOL(1)
            }
        }
        unsafe {
            EnumWindows(Some(enum_window), LPARAM(0_isize))
                .ok()
                .into_diagnostic("factorio::process::could_not_enum_windows")?;
            let max_width = GetSystemMetrics(SM_CXMAXIMIZED);
            let max_height = GetSystemMetrics(SM_CYMAXIMIZED);
            let count = HWNDS.len();
            for (index, window) in HWNDS.iter().enumerate() {
                let (x, y, w, h) = window_size(max_width, max_height, count, index);
                MoveWindow(window, x, y, w, h, BOOL(1)).unwrap();
            }
            HWNDS.clear();
        }
    }
    Ok(())
}

pub fn window_size(
    width_full: i32,
    height_full: i32,
    client_count: usize,
    client_index: usize,
) -> (i32, i32, i32, i32) {
    // cut into two columns and as many rows as needed
    let cols = 2;
    let col_index = (client_index % 2) as i32;
    let rows = (client_count as f64 / 2.0).ceil() as i32;
    let row_index = (client_index / 2) as i32;
    let col_width = width_full / cols;
    let row_height = height_full / rows;
    (
        col_width * col_index,
        row_height * row_index,
        if client_count % 2 == 1 && client_index == client_count - 1 {
            width_full
        } else {
            col_width
        },
        row_height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_size_1() {
        assert_eq!(window_size(800, 600, 1, 0), (0, 0, 800, 600));
    }

    #[test]
    fn test_window_size_2() {
        assert_eq!(window_size(800, 600, 2, 0), (0, 0, 400, 600));
        assert_eq!(window_size(800, 600, 2, 1), (400, 0, 400, 600));
    }

    #[test]
    fn test_window_size_3() {
        assert_eq!(window_size(800, 600, 3, 0), (0, 0, 400, 300));
        assert_eq!(window_size(800, 600, 3, 1), (400, 0, 400, 300));
        assert_eq!(window_size(800, 600, 3, 2), (0, 300, 800, 300));
    }

    #[test]
    fn test_window_size_4() {
        assert_eq!(window_size(800, 600, 4, 0), (0, 0, 400, 300));
        assert_eq!(window_size(800, 600, 4, 1), (400, 0, 400, 300));
        assert_eq!(window_size(800, 600, 4, 2), (0, 300, 400, 300));
        assert_eq!(window_size(800, 600, 4, 3), (400, 300, 400, 300));
    }
}
