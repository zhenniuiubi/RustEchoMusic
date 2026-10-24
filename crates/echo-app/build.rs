//! 构建脚本：把图标与版本信息编进 .exe 资源。
//!
//! gpui 只能改「窗口/任务栏」的图标（运行时 `WM_SETICON`），
//! 资源管理器里那个文件图标是 PE 资源，必须在链接时塞进去。
//! 这里直接用 SDK 的 `rc.exe` 编译一个 `.res` 再交给链接器，不依赖
//! `winres`/`embed-resource`（它们靠读注册表找 SDK，而这个沙箱禁 `reg.exe`）。

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let Some(rc) = find_rc() else {
        println!("cargo:warning=没找到 rc.exe，.exe 文件图标将不会写入资源");
        return;
    };

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let ico = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("MANIFEST_DIR"))
        .join("assets/icon.ico");
    if !ico.exists() {
        println!("cargo:warning=assets/icon.ico 不存在，跳过图标");
        return;
    }
    // rc 里的路径含空格/反斜杠，用正斜杠 + 引号最稳
    let ico_str = ico.to_string_lossy().replace('\\', "/");

    let version = env!("CARGO_PKG_VERSION");
    let mut nums: Vec<&str> = version.split('.').collect();
    while nums.len() < 4 {
        nums.push("0");
    }
    let ver = format!("{},{},{},{}", nums[0], nums[1], nums[2], nums[3]);

    let rc_text = format!(
        r#"#include <winver.h>
IDI_ICON1 ICON "{ico}"

VS_VERSION_INFO VERSIONINFO
 FILEVERSION {ver}
 PRODUCTVERSION {ver}
 FILEFLAGSMASK 0x3fL
 FILEFLAGS 0x0L
 FILEOS 0x40004L
 FILETYPE 0x1L
 FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "080404b0"
        BEGIN
            VALUE "FileDescription", "EchoMusic\0"
            VALUE "FileVersion", "{version}\0"
            VALUE "InternalName", "EchoMusic\0"
            VALUE "OriginalFilename", "EchoMusic.exe\0"
            VALUE "ProductName", "EchoMusic\0"
            VALUE "ProductVersion", "{version}\0"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x804, 1200
    END
END
"#,
        ico = ico_str,
        ver = ver,
        version = version
    );

    let rc_path = out_dir.join("echo_app.rc");
    let res_path = out_dir.join("echo_app.res");
    if std::fs::write(&rc_path, rc_text).is_err() {
        println!("cargo:warning=写 .rc 失败");
        return;
    }

    let status = std::process::Command::new(&rc)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res_path)
        .arg(&rc_path)
        .status();
    match status {
        Ok(s) if s.success() && res_path.exists() => {
            println!("cargo:rustc-link-arg={}", res_path.display());
        }
        Ok(s) => println!("cargo:warning=rc.exe 退出码 {s}，跳过 .exe 图标"),
        Err(e) => println!("cargo:warning=调用 rc.exe 失败：{e}"),
    }
}

/// 找 rc.exe：优先 `RC` 环境变量，其次扫 Windows SDK 的 bin 目录（取最新版本）。
fn find_rc() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RC") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    // 常见位置：C:\Program Files (x86)\Windows Kits\10\bin\<版本>\x64\rc.exe
    let mut roots: Vec<PathBuf> = Vec::new();
    for base in [
        r"C:\Program Files (x86)\Windows Kits\10\bin",
        r"C:\Program Files\Windows Kits\10\bin",
    ] {
        roots.push(PathBuf::from(base));
    }
    let mut best: Option<(Vec<u32>, PathBuf)> = None;
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for e in entries.flatten() {
            let dir = e.path();
            let cand = dir.join("x64").join("rc.exe");
            if !cand.is_file() {
                continue;
            }
            let ver: Vec<u32> = dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .split('.')
                .filter_map(|x| x.parse().ok())
                .collect();
            if best.as_ref().map(|(v, _)| &ver > v).unwrap_or(true) {
                best = Some((ver, cand));
            }
        }
    }
    best.map(|(_, p)| p)
}
