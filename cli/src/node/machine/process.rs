use std::{collections::BTreeMap, ffi::CString, path::PathBuf};

use anyhow::{Result, bail};
use tokio::process::Command;

#[derive(Clone)]
pub struct Identity {
    pub uid: u32,
    pub gid: u32,
    pub name: String,
    pub home: PathBuf,
}

impl Identity {
    pub fn resolve(name: Option<&str>) -> Result<Self> {
        let name = name.map(CString::new).transpose()?;
        let mut record: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result = std::ptr::null_mut();
        let mut buffer = vec![0u8; 65536];
        // SAFETY: buffer and record outlive the reentrant lookup and string copies.
        let status = unsafe {
            match &name {
                Some(name) => libc::getpwnam_r(
                    name.as_ptr(),
                    &mut record,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut result,
                ),
                None => libc::getpwuid_r(
                    libc::geteuid(),
                    &mut record,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut result,
                ),
            }
        };
        if status != 0 || result.is_null() {
            bail!("machine OS user not found");
        }
        let string = |pointer| unsafe {
            std::ffi::CStr::from_ptr(pointer)
                .to_string_lossy()
                .into_owned()
        };
        Ok(Self {
            uid: record.pw_uid,
            gid: record.pw_gid,
            name: string(record.pw_name),
            home: PathBuf::from(string(record.pw_dir)),
        })
    }

    pub fn prepare(&self, command: &mut Command) -> Result<()> {
        let uid = self.uid;
        let gid = self.gid;
        let supervisor = unsafe { libc::geteuid() };
        if supervisor != 0 && supervisor != uid {
            bail!("OS user separation requires the installed supervisor service");
        }
        command.env_clear();
        for (key, value) in std::env::vars_os() {
            let text = key.to_string_lossy();
            if matches!(text.as_ref(), "PATH" | "LANG" | "TMPDIR") || text.starts_with("LC_") {
                command.env(key, value);
            }
        }
        command
            .env(
                "PATH",
                std::env::var_os("PATH").unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".into()),
            )
            .env("HOME", &self.home)
            .env("USER", &self.name)
            .env("LOGNAME", &self.name)
            .env("SHELL", "/bin/sh")
            .env("TERM", "dumb");
        if supervisor == 0 && uid != 0 {
            // SAFETY: only async-signal-safe syscalls in the post-fork child.
            unsafe {
                command.pre_exec(move || {
                    if libc::setgroups(0, std::ptr::null()) != 0
                        || libc::setgid(gid) != 0
                        || libc::setuid(uid) != 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        // Inherited by every descendant: setuid executables and file capabilities
        // cannot restore privileges after the supervisor drops to the worker user.
        #[cfg(target_os = "linux")]
        unsafe {
            command.pre_exec(|| {
                if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        command.process_group(0).kill_on_drop(true);
        Ok(())
    }
}

pub fn request_env(command: &mut Command, values: &BTreeMap<String, String>) -> Result<()> {
    if values.len() > 64 || values.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>() > 16384 {
        bail!("environment limit exceeded");
    }
    for (key, value) in values {
        if key.is_empty()
            || !key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            || key.starts_with("NYXID_")
            || key.starts_with("GIT_CONFIG_")
            || value.contains('\0')
        {
            bail!("environment key is reserved or invalid");
        }
        command.env(key, value);
    }
    Ok(())
}

pub fn pin_cwd(command: &mut Command, directories: Vec<std::fs::File>) {
    use std::os::fd::AsRawFd;
    // Pin the directory before fork. A later path swap cannot redirect the child.
    unsafe {
        command.pre_exec(move || {
            for directory in &directories {
                if libc::faccessat(
                    directory.as_raw_fd(),
                    c".".as_ptr(),
                    libc::X_OK,
                    libc::AT_EACCESS,
                ) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
            }
            let directory = directories
                .last()
                .ok_or_else(|| std::io::Error::from_raw_os_error(libc::ENOENT))?;
            if libc::fchdir(directory.as_raw_fd()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn children_do_not_inherit_node_environment_and_reject_reserved_input() {
        let identity = Identity::resolve(None).unwrap();
        let mut command = Command::new("/usr/bin/env");
        identity.prepare(&mut command).unwrap();
        assert!(
            request_env(
                &mut command,
                &BTreeMap::from([("NYXID_TOKEN".into(), "forbidden".into())])
            )
            .is_err()
        );
        request_env(
            &mut command,
            &BTreeMap::from([("EXAMPLE".into(), "visible".into())]),
        )
        .unwrap();
        let output = command.output().await.unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("EXAMPLE=visible"));
        assert!(!text.contains("NYXID_"));
        assert!(!text.contains("CODEX_"));
    }
}
