// Window capture and OCR calls adapted from the MIT-licensed Azeroth Survivors Trainer.
use image::{RgbaImage, imageops::FilterType};
use std::{io, ptr::null_mut, slice, time::Instant};

use super::panel_text;
use crate::diagnostic_log::event;
use windows::{
    Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::{OcrEngine, OcrResult},
    Storage::Streams::DataWriter,
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, RECT},
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC,
        DeleteObject, SelectObject,
    },
    Storage::Xps::PrintWindow,
    UI::WindowsAndMessaging::{
        EnumWindows, GetClientRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
        IsWindowVisible,
    },
};

struct Frame {
    pixels: Vec<u8>,
    width: usize,
    height: usize,
}

struct Apartment;

impl Apartment {
    fn enter() -> windows::core::Result<Self> {
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }?;
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

pub fn read(pid: u32) -> io::Result<String> {
    let started = Instant::now();
    let frame = capture(pid)?;
    let captured = started.elapsed();
    let _apartment = Apartment::enter().map_err(io::Error::other)?;
    let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(io::Error::other)?;
    let image = RgbaImage::from_raw(frame.width as u32, frame.height as u32, frame.pixels)
        .ok_or_else(|| io::Error::other("Invalid game capture dimensions."))?;
    let (width, height) = image.dimensions();
    let limit = OcrEngine::MaxImageDimension().map_err(io::Error::other)?;
    if width.max(height) > limit {
        return Err(io::Error::other(
            "Game window is too large for Windows OCR.",
        ));
    }
    // Scan the whole game window; the Stats column is located from its heading.
    // Resize only when enlarging: at scale 1 the resize would only cost time.
    let scale = (limit / width.max(height)).clamp(1, 3);
    let scanned = if scale > 1 {
        image::imageops::resize(&image, width * scale, height * scale, FilterType::Lanczos3)
    } else {
        image
    };
    let recognizing = Instant::now();
    let result = recognize(
        &engine,
        scanned.as_raw(),
        scanned.width() as usize,
        scanned.height() as usize,
    )
    .map_err(io::Error::other)?;
    event(
        "ocr.scan",
        format_args!(
            "pid={pid} capture_ms={} recognize_ms={} scale={scale}",
            captured.as_millis(),
            recognizing.elapsed().as_millis()
        ),
    );
    let columns = stats_column(&result, scanned.width() as f32)
        .map_err(io::Error::other)?
        .ok_or_else(|| {
            io::Error::other(
                "Stats panel was not found on the game screen. Open Character in Reborn and try again.",
            )
        })?;
    let rows = stat_rows(&result, Some(columns)).map_err(io::Error::other)?;
    panel_text(&rows).ok_or_else(|| {
        io::Error::other("Stats panel was not recognized. Open Character in Reborn and try again.")
    })
}

fn recognize(
    engine: &OcrEngine,
    pixels: &[u8],
    width: usize,
    height: usize,
) -> windows::core::Result<OcrResult> {
    let writer = DataWriter::new()?;
    writer.WriteBytes(pixels)?;
    let buffer = writer.DetachBuffer()?;
    let bitmap = SoftwareBitmap::CreateWithAlpha(
        BitmapPixelFormat::Bgra8,
        width as i32,
        height as i32,
        BitmapAlphaMode::Ignore,
    )?;
    bitmap.CopyFromBuffer(&buffer)?;
    engine.RecognizeAsync(&bitmap)?.get()
}

struct Word {
    x: f32,
    center_x: f32,
    center: f32,
    height: f32,
    text: String,
}

fn collect_words(result: &OcrResult) -> windows::core::Result<Vec<Word>> {
    let lines = result.Lines()?;
    let mut words = Vec::new();
    for index in 0..lines.Size()? {
        let line = lines.GetAt(index)?.Words()?;
        for index in 0..line.Size()? {
            let word = line.GetAt(index)?;
            let bounds = word.BoundingRect()?;
            words.push(Word {
                x: bounds.X,
                center_x: bounds.X + bounds.Width / 2.0,
                center: bounds.Y + bounds.Height / 2.0,
                height: bounds.Height,
                text: word.Text()?.to_string_lossy(),
            });
        }
    }
    Ok(words)
}

// The Stats heading sits in the middle of its column; the column spans about 8.5% of the
// game width to the left of the heading and 7.5% to the right.
fn stats_column(result: &OcrResult, width: f32) -> windows::core::Result<Option<(f32, f32)>> {
    let words = collect_words(result)?;
    for heading in words
        .iter()
        .filter(|word| word.text.eq_ignore_ascii_case("Stats"))
    {
        let columns = (
            heading.center_x - 0.085 * width,
            heading.center_x + 0.075 * width,
        );
        // Another "Stats" text elsewhere on screen has no OFFENSE and DEFENSE sections under it.
        let has_section = |name: &str| {
            words.iter().any(|word| {
                let inside = (columns.0..=columns.1).contains(&word.center_x);
                inside && word.text.eq_ignore_ascii_case(name)
            })
        };
        if has_section("OFFENSE") && has_section("DEFENSE") {
            return Ok(Some(columns));
        }
    }
    Ok(None)
}

fn stat_rows(result: &OcrResult, columns: Option<(f32, f32)>) -> windows::core::Result<String> {
    let mut words: Vec<Word> = collect_words(result)?
        .into_iter()
        .filter(|word| columns.is_none_or(|(left, right)| (left..=right).contains(&word.center_x)))
        .collect();
    words.sort_by(|a, b| a.center.total_cmp(&b.center));
    let mut text = String::new();
    let mut remaining = words.as_mut_slice();
    while let Some(first) = remaining.first() {
        let end = remaining
            .partition_point(|word| word.center - first.center <= (first.height * 0.5).max(3.0));
        let (row, rest) = remaining.split_at_mut(end);
        row.sort_by(|a, b| a.x.total_cmp(&b.x));
        for word in row {
            text.push_str(&word.text);
            text.push(' ');
        }
        text.push('\n');
        remaining = rest;
    }
    Ok(text)
}

fn game_window(pid: u32) -> HWND {
    struct Search {
        pid: u32,
        hwnd: HWND,
    }
    unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> i32 {
        unsafe {
            let search = &mut *(data as *mut Search);
            let mut owner = 0;
            GetWindowThreadProcessId(hwnd, &mut owner);
            if owner == search.pid && IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 {
                let mut title = [0u16; 64];
                let len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) as usize;
                if String::from_utf16_lossy(&title[..len])
                    .to_ascii_lowercase()
                    .starts_with("azeroth survivors")
                {
                    search.hwnd = hwnd;
                    return 0;
                }
            }
            1
        }
    }
    unsafe {
        let mut search = Search {
            pid,
            hwnd: null_mut(),
        };
        EnumWindows(Some(visit), (&mut search as *mut Search) as LPARAM);
        search.hwnd
    }
}

fn capture(pid: u32) -> io::Result<Frame> {
    let hwnd = game_window(pid);
    unsafe {
        let mut bounds: RECT = std::mem::zeroed();
        if hwnd.is_null() || GetClientRect(hwnd, &mut bounds) == 0 {
            return Err(io::Error::other("Game window is unavailable."));
        }
        let width = bounds.right - bounds.left;
        let height = bounds.bottom - bounds.top;
        if !(200..=4096).contains(&width) || !(200..=4096).contains(&height) {
            return Err(io::Error::other(
                "Game window size is outside the capture range.",
            ));
        }
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = size_of_val(&info.bmiHeader) as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let dc = CreateCompatibleDC(null_mut());
        if dc.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            DeleteDC(dc);
            return Err(io::Error::other("Unable to allocate a game screenshot."));
        }
        let previous = SelectObject(dc, bitmap);
        let frame = if !previous.is_null() && PrintWindow(hwnd, dc, 3) != 0 {
            let stride = width as usize * 4;
            let width = width as usize;
            let height = height as usize;
            let source = slice::from_raw_parts(bits.cast::<u8>(), stride * height);
            let pixels = source.to_vec();
            Ok(Frame {
                pixels,
                width,
                height,
            })
        } else {
            Err(io::Error::other("The game did not return a screenshot."))
        };
        if !previous.is_null() {
            SelectObject(dc, previous);
        }
        DeleteObject(bitmap);
        DeleteDC(dc);
        frame
    }
}
