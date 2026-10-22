use windows::core::{HSTRING, PCWSTR};
use windows::Win32::System::Diagnostics::Debug::*;
use windows::Win32::System::Threading::GetCurrentProcess;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let exe = &args[1];
    let process = unsafe { GetCurrentProcess() };
    let folder = std::path::Path::new(exe).parent().unwrap().to_string_lossy().to_string();
    unsafe {
        SymSetOptions(SYMOPT_LOAD_LINES | SYMOPT_UNDNAME);
        SymInitializeW(process, &HSTRING::from(folder), false).unwrap();
        let base = SymLoadModuleExW(process, None, &HSTRING::from(exe.as_str()), PCWSTR::null(), 0x1_4000_0000, 0, None, None);
        assert!(base != 0, "can't load symbols");
    }
    for offset in &args[2..] {
        let offset = u64::from_str_radix(offset.trim_start_matches("0x"), 16).unwrap();
        let address = 0x1_4000_0000 + offset;
        let mut buffer = vec![0u8; std::mem::size_of::<SYMBOL_INFOW>() + 1024];
        let symbol = buffer.as_mut_ptr() as *mut SYMBOL_INFOW;
        let name = unsafe {
            (*symbol).SizeOfStruct = std::mem::size_of::<SYMBOL_INFOW>() as u32;
            (*symbol).MaxNameLen = 500;
            let mut displacement = 0u64;
            if SymFromAddrW(process, address, Some(&mut displacement), symbol).is_ok() {
                let len = (*symbol).NameLen as usize;
                String::from_utf16_lossy(std::slice::from_raw_parts((*symbol).Name.as_ptr(), len))
            } else {
                "?".into()
            }
        };
        let mut line = IMAGEHLP_LINEW64 { SizeOfStruct: std::mem::size_of::<IMAGEHLP_LINEW64>() as u32, ..Default::default() };
        let mut displacement = 0u32;
        let place = unsafe {
            if SymGetLineFromAddrW64(process, address, &mut displacement, &mut line).is_ok() {
                format!("{}:{}", line.FileName.to_string().unwrap_or_default(), line.LineNumber)
            } else {
                String::new()
            }
        };
        println!("+0x{offset:x}  {name}  {place}");
    }
}
