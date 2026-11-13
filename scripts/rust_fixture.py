"""Prerequisites for optional local Rust fixtures and mandatory CI executions.

Only query rustup's installed inventory. Real commands use `rustup run` without
`--install`, so a fixture can never provision its own compiler.
"""
import functools
import os
import shutil
import subprocess

TOOLCHAIN = '1.98.1'


def missing_prerequisite(*, components=(), tools=()):
    if os.environ.get('NYXID_SKIP_RUST_FIXTURES') == '1':
        return 'Rust compilation fixtures disabled in this job'
    for tool in ('rustup', 'cargo', 'rustc', *tools):
        if not shutil.which(tool):
            return f'requires installed {tool}'
    try:
        installed = subprocess.check_output(
            ['rustup', 'toolchain', 'list', '--quiet'], text=True, stderr=subprocess.PIPE, timeout=5)
        names = [line.split()[0] for line in installed.splitlines() if line.strip()]
        if not any(name == TOOLCHAIN or name.startswith(TOOLCHAIN + '-') for name in names):
            return f'requires installed Rust {TOOLCHAIN}'
        installed = subprocess.check_output(
            ['rustup', 'component', 'list', '--toolchain', TOOLCHAIN, '--installed'],
            text=True, stderr=subprocess.PIPE, timeout=5)
        names = installed.splitlines()
        for component in ('cargo', 'rustc', 'rust-std', *components):
            if not any(name == component or name.startswith(component + '-') for name in names):
                return f'requires installed {component} for Rust {TOOLCHAIN}'
    except (OSError, subprocess.SubprocessError) as error:
        return f'cannot inspect installed Rust prerequisites: {error}'
    return None


def requires_rust(*, components=(), tools=()):
    def decorate(test):
        @functools.wraps(test)
        def checked(self, *args, **kwargs):
            reason = missing_prerequisite(components=components, tools=tools)
            if reason:
                if os.environ.get('NYXID_REQUIRE_RUST_FIXTURES') == '1':
                    self.fail(reason)
                self.skipTest(reason)
            return test(self, *args, **kwargs)
        return checked
    return decorate


def rust_command(tool, *args):
    return ['rustup', 'run', TOOLCHAIN, tool, *args]


def rust_environment():
    return {**os.environ, 'RUSTUP_TOOLCHAIN': TOOLCHAIN, 'RUSTUP_AUTO_INSTALL': '0',
            'CARGO_NET_OFFLINE': 'true'}
