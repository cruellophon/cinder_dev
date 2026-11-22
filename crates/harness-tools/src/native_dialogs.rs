use std::path::Path;

#[cfg(windows)]
fn strip_verbatim(p: &Path) -> String {
    let s = p.to_string_lossy();
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
}

#[cfg(windows)]
pub mod win32 {
    use std::ffi::c_void;
    use std::ptr;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    const CLSID_FILE_OPEN_DIALOG: Guid = Guid {
        data1: 0xDC1C5A9C,
        data2: 0xE88A,
        data3: 0x4DDE,
        data4: [0xA5, 0xA1, 0x60, 0xF8, 0x2A, 0x20, 0xAE, 0xF7],
    };
    const IID_I_FILE_OPEN_DIALOG: Guid = Guid {
        data1: 0xD57C7288,
        data2: 0xD4AD,
        data3: 0x4768,
        data4: [0xBE, 0x02, 0x9D, 0x96, 0x95, 0x32, 0xD9, 0x60],
    };

    const COINIT_APARTMENTTHREADED_NO_OLE1: u32 = 0x2 | 0x4;
    const CLSCTX_INPROC_SERVER: u32 = 0x1;
    const FOS_PICKFOLDERS: u32 = 0x0000_0020;
    const FOS_FORCEFILESYSTEM: u32 = 0x0000_0040;
    const FOS_NOCHANGEDIR: u32 = 0x0000_0008;
    const FOS_ALLOWMULTISELECT: u32 = 0x0000_0200;
    const FOS_PATHMUSTEXIST: u32 = 0x0000_0800;
    const FOS_FILEMUSTEXIST: u32 = 0x0000_1000;
    const SIGDN_FILESYSPATH: u32 = 0x8005_8000;
    const HRESULT_CANCELLED: i32 = 0x8007_04C7u32 as i32;

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(pvReserved: *mut c_void, dwCoInit: u32) -> i32;
        fn CoUninitialize();
        fn CoCreateInstance(
            rclsid: *const Guid,
            pUnkOuter: *mut c_void,
            dwClsContext: u32,
            riid: *const Guid,
            ppv: *mut *mut c_void,
        ) -> i32;
        fn CoTaskMemFree(pv: *mut c_void);
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetForegroundWindow() -> isize;
        fn EnumWindows(lpEnumFunc: extern "system" fn(isize, isize) -> i32, lParam: isize) -> i32;
        fn IsWindowVisible(hwnd: isize) -> i32;
        fn GetWindowTextW(hwnd: isize, lpString: *mut u16, nMaxCount: i32) -> i32;
        fn SetForegroundWindow(hwnd: isize) -> i32;
    }

    extern "system" fn enum_find_cinder(hwnd: isize, lparam: isize) -> i32 {
        unsafe {
            if IsWindowVisible(hwnd) == 0 {
                return 1;
            }
            let mut buf = [0u16; 128];
            let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            if len > 0 {
                let title = String::from_utf16_lossy(&buf[..len as usize]);
                if title.contains("Cinder") {
                    let out = &mut *(lparam as *mut isize);
                    *out = hwnd;
                    return 0;
                }
            }
            1
        }
    }

    fn find_owner_hwnd() -> isize {
        let mut found: isize = 0;
        unsafe {
            EnumWindows(enum_find_cinder, (&mut found as *mut isize) as isize);
            if found != 0 {
                let _ = SetForegroundWindow(found);
                return found;
            }
            GetForegroundWindow()
        }
    }

    #[inline]
    unsafe fn vtbl(obj: *mut c_void) -> *const *const c_void {
        *(obj as *const *const *const c_void)
    }

    pub struct ComGuard(bool);
    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }

    pub struct ComPtr(pub *mut c_void);
    impl Drop for ComPtr {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    let release: extern "system" fn(*mut c_void) -> u32 =
                        std::mem::transmute(*vtbl(self.0).add(2));
                    release(self.0);
                }
            }
        }
    }

    pub struct DialogSession {
        pub dlg: ComPtr,
        pub _com: ComGuard,
    }

    pub fn create_dialog(title: &str, extra_flags: u32) -> anyhow::Result<DialogSession> {
        unsafe {
            let hr_init = CoInitializeEx(ptr::null_mut(), COINIT_APARTMENTTHREADED_NO_OLE1);
            let should_uninit = hr_init == 0 || hr_init == 1;
            if hr_init < 0 {
                anyhow::bail!("COM CoInitializeEx failed: 0x{:08X}", hr_init as u32);
            }
            let com = ComGuard(should_uninit);

            let mut raw_dlg: *mut c_void = ptr::null_mut();
            let hr_create = CoCreateInstance(
                &CLSID_FILE_OPEN_DIALOG,
                ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_I_FILE_OPEN_DIALOG,
                &mut raw_dlg,
            );
            if hr_create < 0 || raw_dlg.is_null() {
                anyhow::bail!(
                    "CoCreateInstance(IFileOpenDialog) failed: 0x{:08X}",
                    hr_create as u32
                );
            }
            let dlg = ComPtr(raw_dlg);

            let get_opts: extern "system" fn(*mut c_void, *mut u32) -> i32 =
                std::mem::transmute(*vtbl(dlg.0).add(10));
            let set_opts: extern "system" fn(*mut c_void, u32) -> i32 =
                std::mem::transmute(*vtbl(dlg.0).add(9));
            let set_title: extern "system" fn(*mut c_void, *const u16) -> i32 =
                std::mem::transmute(*vtbl(dlg.0).add(17));

            let mut opts: u32 = 0;
            let _ = get_opts(dlg.0, &mut opts);
            let _ = set_opts(
                dlg.0,
                opts | FOS_FORCEFILESYSTEM | FOS_NOCHANGEDIR | extra_flags,
            );

            let wide_title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
            let _ = set_title(dlg.0, wide_title.as_ptr());

            Ok(DialogSession { dlg, _com: com })
        }
    }

    pub fn create_folder_dialog(title: &str) -> anyhow::Result<DialogSession> {
        create_dialog(title, FOS_PICKFOLDERS)
    }

    unsafe fn shell_item_path(item: *mut c_void) -> Option<String> {
        let get_name: extern "system" fn(*mut c_void, u32, *mut *mut u16) -> i32 =
            std::mem::transmute(*vtbl(item).add(5));
        let mut pwstr: *mut u16 = ptr::null_mut();
        let hr_name = get_name(item, SIGDN_FILESYSPATH, &mut pwstr);
        if hr_name < 0 || pwstr.is_null() {
            return None;
        }
        let mut len = 0usize;
        while *pwstr.add(len) != 0 {
            len += 1;
        }
        let path = String::from_utf16_lossy(std::slice::from_raw_parts(pwstr, len));
        CoTaskMemFree(pwstr as *mut c_void);
        let trimmed = path.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    pub fn pick_folder(title: &str) -> anyhow::Result<Option<String>> {
        let session = create_folder_dialog(title)?;
        let hwnd = find_owner_hwnd();
        unsafe {
            let show: extern "system" fn(*mut c_void, isize) -> i32 =
                std::mem::transmute(*vtbl(session.dlg.0).add(3));
            let hr_show = show(session.dlg.0, hwnd);
            if hr_show == HRESULT_CANCELLED {
                return Ok(None);
            }
            if hr_show < 0 {
                anyhow::bail!("IFileOpenDialog::Show failed: 0x{:08X}", hr_show as u32);
            }

            let get_result: extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 =
                std::mem::transmute(*vtbl(session.dlg.0).add(20));
            let mut raw_item: *mut c_void = ptr::null_mut();
            let hr_res = get_result(session.dlg.0, &mut raw_item);
            if hr_res < 0 || raw_item.is_null() {
                return Ok(None);
            }
            let item = ComPtr(raw_item);
            Ok(shell_item_path(item.0))
        }
    }

    pub fn pick_files(title: &str) -> anyhow::Result<Vec<String>> {
        let session = create_dialog(
            title,
            FOS_ALLOWMULTISELECT | FOS_PATHMUSTEXIST | FOS_FILEMUSTEXIST,
        )?;
        let hwnd = find_owner_hwnd();
        unsafe {
            let show: extern "system" fn(*mut c_void, isize) -> i32 =
                std::mem::transmute(*vtbl(session.dlg.0).add(3));
            let hr_show = show(session.dlg.0, hwnd);
            if hr_show == HRESULT_CANCELLED {
                return Ok(Vec::new());
            }
            if hr_show < 0 {
                anyhow::bail!("IFileOpenDialog::Show failed: 0x{:08X}", hr_show as u32);
            }

            // Try GetResults (vtable slot 27 on IFileOpenDialog) -> IShellItemArray
            let get_results: extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 =
                std::mem::transmute(*vtbl(session.dlg.0).add(27));
            let mut raw_arr: *mut c_void = ptr::null_mut();
            let hr_arr = get_results(session.dlg.0, &mut raw_arr);
            if hr_arr >= 0 && !raw_arr.is_null() {
                let arr = ComPtr(raw_arr);
                let get_count: extern "system" fn(*mut c_void, *mut u32) -> i32 =
                    std::mem::transmute(*vtbl(arr.0).add(7));
                let get_item_at: extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32 =
                    std::mem::transmute(*vtbl(arr.0).add(8));
                let mut count: u32 = 0;
                if get_count(arr.0, &mut count) >= 0 {
                    let mut paths = Vec::with_capacity(count as usize);
                    for idx in 0..count {
                        let mut raw_item: *mut c_void = ptr::null_mut();
                        if get_item_at(arr.0, idx, &mut raw_item) >= 0 && !raw_item.is_null() {
                            let item = ComPtr(raw_item);
                            if let Some(p) = shell_item_path(item.0) {
                                paths.push(p);
                            }
                        }
                    }
                    return Ok(paths);
                }
            }

            // Single-item fallback via GetResult (slot 20)
            let get_result: extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 =
                std::mem::transmute(*vtbl(session.dlg.0).add(20));
            let mut raw_item: *mut c_void = ptr::null_mut();
            if get_result(session.dlg.0, &mut raw_item) >= 0 && !raw_item.is_null() {
                let item = ComPtr(raw_item);
                if let Some(p) = shell_item_path(item.0) {
                    return Ok(vec![p]);
                }
            }
            Ok(Vec::new())
        }
    }
}

#[cfg(any(target_os = "macos", test))]
fn escape_applescript_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Open the native OS folder chooser on a dedicated STA thread (safe alongside winit/eframe).
pub fn pick_folder(title: &str) -> anyhow::Result<Option<String>> {
    #[cfg(windows)]
    {
        let title_owned = title.to_string();
        let handle = std::thread::spawn(move || win32::pick_folder(&title_owned));
        return handle
            .join()
            .map_err(|_| anyhow::anyhow!("Folder picker UI thread panicked"))?;
    }
    #[cfg(target_os = "macos")]
    {
        let escaped = escape_applescript_string(title);
        let out = std::process::Command::new("osascript")
            .args([
                "-e",
                &format!("POSIX path of (choose folder with prompt \"{escaped}\")"),
            ])
            .output()?;
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        return Ok(if s.is_empty() { None } else { Some(s) });
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let out = match std::process::Command::new("zenity")
            .args(["--file-selection", "--directory", &format!("--title={title}")])
            .output()
        {
            Ok(out) => out,
            Err(_) => std::process::Command::new("kdialog")
                .args(["--getexistingdirectory", ".", "--title", title])
                .output()?,
        };
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        return Ok(if s.is_empty() { None } else { Some(s) });
    }
}

/// Open the native OS multi-file chooser on a dedicated STA thread.
pub fn pick_files(title: &str) -> anyhow::Result<Vec<String>> {
    #[cfg(windows)]
    {
        let title_owned = title.to_string();
        let handle = std::thread::spawn(move || win32::pick_files(&title_owned));
        return handle
            .join()
            .map_err(|_| anyhow::anyhow!("File picker UI thread panicked"))?;
    }
    #[cfg(target_os = "macos")]
    {
        let escaped = escape_applescript_string(title);
        let script = format!(
            "set picked to choose file with prompt \"{escaped}\" with multiple selections allowed\n\
             set outList to \"\"\n\
             repeat with f in picked\n\
               set outList to outList & (POSIX path of f) & linefeed\n\
             end repeat\n\
             return outList"
        );
        let out = std::process::Command::new("osascript")
            .args(["-e", &script])
            .output()?;
        let text = String::from_utf8_lossy(&out.stdout);
        return Ok(text
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        match std::process::Command::new("zenity")
            .args(["--file-selection", "--multiple", &format!("--title={title}")])
            .output()
        {
            Ok(out) => {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                return Ok(if s.is_empty() {
                    Vec::new()
                } else {
                    s.split('|')
                        .map(str::trim)
                        .filter(|x| !x.is_empty())
                        .map(str::to_string)
                        .collect()
                });
            }
            Err(_) => {
                let out = std::process::Command::new("kdialog")
                    .args([
                        "--getopenfilename",
                        ".",
                        "--multiple",
                        "--separate-output",
                        "--title",
                        title,
                    ])
                    .output()?;
                let text = String::from_utf8_lossy(&out.stdout);
                return Ok(text
                    .lines()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect());
            }
        }
    }
}

/// Highlight a file or open a folder in the OS file manager (Windows Explorer / Finder / xdg-open).
pub fn reveal_in_file_manager(path: &Path) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        let clean = strip_verbatim(path);
        let mut cmd = std::process::Command::new("explorer.exe");
        if path.is_dir() {
            cmd.arg(&clean);
        } else {
            cmd.arg(format!("/select,{clean}"));
        }
        cmd.spawn()?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &path.to_string_lossy()])
            .spawn()?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let target = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        std::process::Command::new("xdg-open").arg(target).spawn()?;
        return Ok(());
    }
}

/// Open a file, HTML preview, PDF, or URL with the OS default handler.
pub fn open_with_system_default(path_or_url: &Path) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let clean = strip_verbatim(path_or_url);
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &clean])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path_or_url)
            .spawn()?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(path_or_url)
            .spawn()?;
        return Ok(());
    }
}

/// Safely open an external web/mailto URL with the OS default browser without shell interpretation.
pub fn open_url_safely(raw_url: &str) -> anyhow::Result<()> {
    let parsed = url::Url::parse(raw_url.trim())
        .map_err(|e| anyhow::anyhow!("Invalid URL '{raw_url}': {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https" | "mailto") {
        anyhow::bail!("Unsupported URL scheme '{}'", parsed.scheme());
    }
    let clean = parsed.as_str();

    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        extern "system" {
            fn ShellExecuteW(
                hwnd: isize,
                lp_operation: *const u16,
                lp_file: *const u16,
                lp_parameters: *const u16,
                lp_directory: *const u16,
                n_show_cmd: i32,
            ) -> isize;
        }
        let op: Vec<u16> = "open\0".encode_utf16().collect();
        let file: Vec<u16> = clean.encode_utf16().chain(std::iter::once(0)).collect();
        let rc = unsafe {
            ShellExecuteW(
                0,
                op.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        };
        if rc <= 32 {
            anyhow::bail!("Windows ShellExecuteW could not open URL (code {rc})");
        }
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(clean).spawn()?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(clean).spawn()?;
        return Ok(());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn open_url_safely_rejects_dangerous_schemes() {
        for bad in [
            "javascript:alert(1)",
            "file:///C:/Windows/system32/cmd.exe",
            "cmd:/c calc",
            "not a url",
        ] {
            assert!(
                super::open_url_safely(bad).is_err(),
                "should reject '{bad}'"
            );
        }
    }

    #[test]
    fn escape_applescript_string_escapes_quotes_and_backslashes() {
        assert_eq!(
            super::escape_applescript_string(r#"Pick "Project" in C:\Work"#),
            r#"Pick \"Project\" in C:\\Work"#
        );
    }

    #[cfg(windows)]
    #[test]
    fn win32_dialogs_initialize_on_sta_thread() {
        let ok_folder = std::thread::spawn(|| {
            super::win32::create_folder_dialog("Cinder Folder Test").is_ok()
        })
        .join()
        .unwrap();
        assert!(ok_folder);

        let ok_files = std::thread::spawn(|| {
            super::win32::create_dialog("Cinder File Test", 0x0000_0200).is_ok()
        })
        .join()
        .unwrap();
        assert!(ok_files);
    }
}
