//! 音频线程实时优先级提升。
//!
//! - Windows：MMCSS 的 "Pro Audio" 任务（`avrt!AvSetMmThreadCharacteristicsW`），
//!   这是所有正经音频引擎的做法；纯 FFI，不引入额外 crate。
//! - Linux：`pthread_setschedparam(SCHED_FIFO)`，失败通常是缺 `rtprio` 权限，
//!   把原因返回给调用方，由**非实时线程**打印提示（回调里禁止日志）。
//!
//! 调用点：音频回调**第一次**进入时提升（线程由 cpal 内部创建，我们拿不到句柄）。
//! 提升本身只做一次，之后走 `AtomicBool` 分支。

/// 在当前线程上尝试提升到实时优先级。
pub fn promote_current_thread() -> Result<(), String> {
    imp::promote()
}

#[cfg(windows)]
mod imp {
    //! `avrt.dll` 里的 MMCSS 接口。
    //!
    //! 这些符号属于 Win32 的实时任务服务，`windows`/`winapi` crate 都有封装，
    //! 但为了不新增依赖（当前环境无法访问 crates.io），这里直接声明。

    #[link(name = "avrt")]
    unsafe extern "system" {
        fn AvSetMmThreadCharacteristicsW(task_name: *const u16, task_index: *mut u32) -> isize;
    }

    /// "Pro Audio\0" 的 UTF-16 常量，避免在实时线程里分配。
    const PRO_AUDIO: [u16; 10] = [0x50, 0x72, 0x6F, 0x20, 0x41, 0x75, 0x64, 0x69, 0x6F, 0x00];

    pub fn promote() -> Result<(), String> {
        let mut task_index: u32 = 0;
        // 返回值是 AvRevertMmThreadCharacteristics 需要的句柄；非 0 即成功。
        let handle = unsafe { AvSetMmThreadCharacteristicsW(PRO_AUDIO.as_ptr(), &mut task_index) };
        if handle == 0 {
            let code = std::io::Error::last_os_error();
            return Err(format!("AvSetMmThreadCharacteristics 失败: {code}"));
        }
        // 句柄不主动 revert：线程退出时系统会回收，且我们整个进程生命周期都需要它。
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod imp {
    /// `SCHED_FIFO` 调度策略。
    const SCHED_FIFO: i32 = 1;

    #[repr(C)]
    struct SchedParam {
        sched_priority: i32,
    }

    unsafe extern "C" {
        fn pthread_self() -> usize;
        fn pthread_setschedparam(thread: usize, policy: i32, param: *const SchedParam) -> i32;
    }

    pub fn promote() -> Result<(), String> {
        let param = SchedParam {
            sched_priority: 80,
        };
        let rc = unsafe { pthread_setschedparam(pthread_self(), SCHED_FIFO, &param) };
        if rc != 0 {
            return Err(format!(
                "pthread_setschedparam(SCHED_FIFO, 80) 失败 rc={rc}；\
                 需要 CAP_SYS_NICE 或在 /etc/security/limits.conf 给用户加 rtprio"
            ));
        }
        Ok(())
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    pub fn promote() -> Result<(), String> {
        Err("当前平台未实现音频线程实时优先级".to_string())
    }
}
