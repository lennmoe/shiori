use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HANDLE, HMODULE};
use windows::Win32::System::Diagnostics::Debug::{
    MiniDumpWithIndirectlyReferencedMemory, MiniDumpWithProcessThreadData, MiniDumpWithThreadInfo,
    MiniDumpWithUnloadedModules, MiniDumpWriteDump, RtlCaptureStackBackTrace,
    SetUnhandledExceptionFilter, EXCEPTION_POINTERS, MINIDUMP_EXCEPTION_INFORMATION,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId};

struct Target {
    dir: PathBuf,
    version: String,
}

static TARGET: OnceLock<Target> = OnceLock::new();
static TRAIL: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
const TRAIL_LEN: usize = 60;

pub fn note(text: impl AsRef<str>) {
    let thread = std::thread::current();
    let line = format!(
        "{} [{}] {}",
        chrono::Local::now().format("%H:%M:%S%.3f"),
        thread.name().unwrap_or("?"),
        text.as_ref(),
    );
    if let Ok(mut trail) = TRAIL.lock() {
        if trail.len() == TRAIL_LEN {
            trail.pop_front();
        }
        trail.push_back(line);
    }
}

pub fn event(text: impl AsRef<str>) {
    note(text.as_ref());
    append_event(text.as_ref());
}

fn append_event(text: &str) {
    let Some(target) = TARGET.get() else {
        return;
    };
    let file = target.dir.join("events.log");
    if std::fs::metadata(&file).is_ok_and(|meta| meta.len() > 256 * 1024) {
        let _ = std::fs::rename(&file, target.dir.join("events.old.log"));
    }
    let line = format!(
        "{} v{} {text}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        target.version,
    );
    if let Ok(mut log) = OpenOptions::new().create(true).append(true).open(file) {
        let _ = log.write_all(line.as_bytes());
    }
}

pub fn started() {
    let Some(target) = TARGET.get() else {
        return;
    };
    let previous = std::fs::read_to_string(target.dir.join("events.log")).unwrap_or_default();
    if let Some(last) = previous.lines().last() {
        if !last.ends_with(" exit") && !last.contains("(see crash.log)") {
            event("the previous run ended without closing normally");
        }
    }
    event("start");
}

fn trail_text() -> String {
    match TRAIL.try_lock() {
        Ok(trail) if !trail.is_empty() => {
            let lines: Vec<String> = trail.iter().map(|line| format!("    {line}")).collect();
            format!("\n  last actions:\n{}", lines.join("\n"))
        }
        _ => String::new(),
    }
}

pub fn install(dir: PathBuf, version: String) {
    if TARGET.set(Target { dir, version }).is_err() {
        return;
    }
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        write_line(&format!("[{}] {info}{}", thread.name().unwrap_or("?"), trail_text()));
        append_event("panic (see crash.log)");
    }));
    unsafe {
        SetUnhandledExceptionFilter(Some(on_exception));
    }
}

fn write_line(text: &str) {
    let Some(target) = TARGET.get() else {
        return;
    };
    let line = format!(
        "{} v{} {text}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        target.version,
    );
    if let Ok(mut log) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(target.dir.join("crash.log"))
    {
        let _ = log.write_all(line.as_bytes());
    }
}

fn module_of(address: usize) -> String {
    let mut module = HMODULE::default();
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(address as *const u16),
            &mut module,
        )
    };
    if found.is_err() {
        return format!("0x{address:x}");
    }
    let mut name = [0u16; 260];
    let len = unsafe { GetModuleFileNameW(Some(module), &mut name) } as usize;
    let path = String::from_utf16_lossy(&name[..len]);
    let file = path.rsplit('\\').next().unwrap_or(&path).to_string();
    format!("{file}+0x{:x}", address - module.0 as usize)
}

unsafe extern "system" fn on_exception(info: *const EXCEPTION_POINTERS) -> i32 {
    let Some(target) = TARGET.get() else {
        return 0;
    };
    let record = unsafe { &*(*info).ExceptionRecord };
    let code = record.ExceptionCode.0 as u32;
    let thread = std::thread::current();

    let mut frames = [std::ptr::null_mut(); 48];
    let count = unsafe { RtlCaptureStackBackTrace(0, &mut frames, None) } as usize;
    let stack: Vec<String> = frames[..count]
        .iter()
        .map(|frame| format!("    {}", module_of(*frame as usize)))
        .collect();

    append_event(&format!("native crash 0x{code:08X} (see crash.log)"));
    write_line(&format!(
        "[{}] native crash 0x{code:08X} at {}\n{}{}",
        thread.name().unwrap_or("?"),
        module_of(record.ExceptionAddress as usize),
        stack.join("\n"),
        trail_text(),
    ));

    if let Ok(file) = File::create(target.dir.join("crash.dmp")) {
        let exception = MINIDUMP_EXCEPTION_INFORMATION {
            ThreadId: unsafe { GetCurrentThreadId() },
            ExceptionPointers: info as *mut _,
            ClientPointers: false.into(),
        };
        unsafe {
            let _ = MiniDumpWriteDump(
                GetCurrentProcess(),
                GetCurrentProcessId(),
                HANDLE(file.as_raw_handle()),
                MiniDumpWithThreadInfo
                    | MiniDumpWithIndirectlyReferencedMemory
                    | MiniDumpWithUnloadedModules
                    | MiniDumpWithProcessThreadData,
                Some(&exception),
                None,
                None,
            );
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panics_and_modules_are_logged() {
        let dir = std::env::temp_dir().join("vnrpc_crash_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        install(dir.clone(), "9.9.9".into());
        note("opened the mascot menu");

        let crashed = std::thread::Builder::new()
            .name("mascot-test".into())
            .spawn(|| panic!("test crash"))
            .unwrap()
            .join();
        let _ = std::panic::take_hook();
        assert!(crashed.is_err());

        let log = std::fs::read_to_string(dir.join("crash.log")).unwrap();
        assert!(log.contains("v9.9.9 [mascot-test]"), "{log}");
        assert!(log.contains("test crash"), "{log}");
        assert!(log.contains("last actions:") && log.contains("opened the mascot menu"), "{log}");

        let here = module_of(panics_and_modules_are_logged as *const () as usize);
        assert!(here.contains(".exe+0x"), "{here}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
