use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use windows::core::{Interface, GUID, HSTRING, PCWSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Pictures, IShellLinkW, SHFileOperationW, SHGetKnownFolderPath,
    SetCurrentProcessExplicitAppUserModelID, ShellExecuteW, ShellLink, KNOWN_FOLDER_FLAG,
    SHFILEOPSTRUCTW,
};

pub const APP_ID: &str = "moe.lenn.shiori";
const SHORTCUT_APP_ID: &str = "moe.lenn.shiori.play";

pub fn set_app_id() {
    unsafe {
        let _ = SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(APP_ID));
    }
}
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

pub fn open(target: &str) -> Result<(), String> {
    shell_execute(target, "", None)
}

pub fn start_program(program: &Path, params: &str, cwd: &Path) -> Result<(), String> {
    shell_execute(&program.to_string_lossy(), params, Some(cwd))
}

fn shell_execute(target: &str, params: &str, cwd: Option<&Path>) -> Result<(), String> {
    let target = HSTRING::from(target);
    let params = HSTRING::from(params);
    let cwd = HSTRING::from(
        cwd.map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
    );

    let result = unsafe {
        ShellExecuteW(
            None,
            &HSTRING::from("open"),
            &target,
            if params.is_empty() {
                PCWSTR::null()
            } else {
                PCWSTR(params.as_ptr())
            },
            if cwd.is_empty() {
                PCWSTR::null()
            } else {
                PCWSTR(cwd.as_ptr())
            },
            SW_SHOWNORMAL,
        )
    };

    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!(
            "Windows couldn't open it (error {})",
            result.0 as isize
        ))
    }
}

pub fn show_in_folder(path: &Path) -> Result<(), String> {
    Command::new("explorer")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn send_to_recycle_bin(path: &Path) -> Result<(), String> {
    const FO_DELETE: u32 = 3;
    const FOF_SILENT: u16 = 0x4;
    const FOF_NOCONFIRMATION: u16 = 0x10;
    const FOF_ALLOWUNDO: u16 = 0x40;
    const FOF_NOERRORUI: u16 = 0x400;

    let mut from: Vec<u16> = path.to_string_lossy().encode_utf16().collect();
    from.extend([0, 0]);

    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from.as_ptr()),
        fFlags: FOF_SILENT | FOF_NOCONFIRMATION | FOF_ALLOWUNDO | FOF_NOERRORUI,
        ..Default::default()
    };

    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 || op.fAnyOperationsAborted.as_bool() {
        return Err(format!(
            "couldn't move it to the Recycle Bin (error {code})"
        ));
    }
    Ok(())
}

fn known_folder(id: &GUID, fallback: &str) -> PathBuf {
    unsafe {
        if let Ok(ptr) = SHGetKnownFolderPath(id, KNOWN_FOLDER_FLAG(0), None) {
            let path = ptr.to_string().unwrap_or_default();
            CoTaskMemFree(Some(ptr.0 as *const _));
            if !path.is_empty() {
                return PathBuf::from(path);
            }
        }
    }

    let home = std::env::var("USERPROFILE").unwrap_or_default();
    PathBuf::from(home).join(fallback)
}

pub fn pictures_folder() -> PathBuf {
    known_folder(&FOLDERID_Pictures, "Pictures")
}

pub fn desktop_folder() -> PathBuf {
    known_folder(&FOLDERID_Desktop, "Desktop")
}

pub struct Shortcut<'a> {
    pub target: &'a Path,
    pub arguments: &'a str,
    pub icon: &'a Path,
    pub description: &'a str,
}

pub fn create_shortcut(dest: &Path, shortcut: &Shortcut) -> Result<(), String> {
    let folder = shortcut.target.parent().unwrap_or(Path::new("."));
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        link.SetPath(&HSTRING::from(shortcut.target.as_os_str()))
            .and_then(|_| link.SetArguments(&HSTRING::from(shortcut.arguments)))
            .and_then(|_| link.SetWorkingDirectory(&HSTRING::from(folder.as_os_str())))
            .and_then(|_| link.SetIconLocation(&HSTRING::from(shortcut.icon.as_os_str()), 0))
            .and_then(|_| link.SetDescription(&HSTRING::from(shortcut.description)))
            .map_err(|e| e.to_string())?;
        let properties: IPropertyStore = link.cast().map_err(|e| e.to_string())?;
        properties
            .SetValue(&PKEY_AppUserModel_ID, &PROPVARIANT::from(SHORTCUT_APP_ID))
            .and_then(|_| properties.Commit())
            .map_err(|e| e.to_string())?;
        let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
        file.Save(&HSTRING::from(dest.as_os_str()), true)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_get_their_own_app_id() {
        let dest = std::env::temp_dir().join("vnrpc_shortcut_test.lnk");
        let exe = std::env::current_exe().unwrap();
        let shortcut = Shortcut {
            target: &exe,
            arguments: "--play \"test\"",
            icon: &exe,
            description: "test",
        };
        create_shortcut(&dest, &shortcut).unwrap();
        let bytes = std::fs::read(&dest).unwrap();
        let _ = std::fs::remove_file(&dest);
        let wide: Vec<u8> = SHORTCUT_APP_ID.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert!(bytes.windows(wide.len()).any(|w| w == wide.as_slice()));
    }
}
