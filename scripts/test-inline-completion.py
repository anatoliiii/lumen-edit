#!/usr/bin/env python3
"""Explicit cloud + native GUI acceptance on synthetic Rust and Python only.

Requires Linux, Xvfb, xdotool, and a working OpenRouter key in Pi or environment.
Never enables completion in the user's config. Temporary project/state; at most
four provider requests, no private source. Images are written to /tmp.

python3 scripts/test-inline-completion.py --real-api [target/debug/lumen-edit]
Ordinary model-free tests: cargo test --locked --workspace
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--real-api', action='store_true', required=True,
                        help='Explicitly permit paid OpenRouter calls on the synthetic fixtures')
    parser.add_argument('binary', nargs='?', default='target/debug/lumen-edit')
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    for tool in ('Xvfb', 'xdotool'):
        if not shutil.which(tool):
            parser.error(f'{tool} is required')
    pi_dir = Path(os.environ.get('PI_CODING_AGENT_DIR', str(Path.home() / '.pi/agent'))).expanduser().resolve()
    with tempfile.TemporaryDirectory(prefix='lumen-inline-acceptance-') as temporary:
        root = Path(temporary)
        project = root / 'project'
        config = root / 'config/lumen-edit'
        runtime = root / 'runtime'
        for folder in (project, config, runtime, root / 'home'):
            folder.mkdir(parents=True, exist_ok=True)
        runtime.chmod(0o700)
        fixtures = [('square.rs', 'fn sum_squares(values: &[i32]) -> i32 {\n    // Use an explicit for loop and return the accumulated sum.\n   \n}\n', 3, 4),
                    ('square.py', 'def square(x: int) -> int:\n   \n', 2, 4)]
        for filename, source, _, _ in fixtures:
            (project / filename).write_text(source)
        # No LSP binaries are required for this fallback/native-rendering test.
        # The separate semantic fixture test exercises actual JSON-RPC data.
        disabled = ['rust-analyzer', 'clangd', 'gopls', 'ty', 'ruff', 'pylsp',
                    'csharp-ls', 'omnisharp', 'tsgo', 'biome', 'superhtml', 'taplo', 'sourcekit-lsp']
        (config / 'config.toml').write_text(
            'language = "en"\npulse = false\nauto_save = false\n'
            + f'disabled_servers = {json.dumps(disabled)}\n'
            + '[inline_completion]\n'
            + f'projects = [{json.dumps("local|" + str(project))}]\n'
            + 'requests_per_session = 4\nsession_budget_usd = 0.1\n')
        env = dict(os.environ, HOME=str(root / 'home'), XDG_CONFIG_HOME=str(root / 'config'),
                   XDG_RUNTIME_DIR=str(runtime), XDG_CACHE_HOME=str(root / 'cache'),
                   XDG_STATE_HOME=str(root / 'state'), PI_CODING_AGENT_DIR=str(pi_dir), SHELL='/bin/sh')
        for name in ('WAYLAND_DISPLAY', 'LUMEN_LOG', 'LUMEN_DEBUG_VIEW', 'LUMEN_SCREENSHOT'):
            env.pop(name, None)
        with (root / 'editor.log').open('w') as log:
            read_fd, write_fd = os.pipe()
            xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(write_fd), '-screen', '0', '1440x1000x24', '-nolisten', 'tcp', '-ac'],
                                    pass_fds=(write_fd,), stdout=log, stderr=log)
            os.close(write_fd)
            with os.fdopen(read_fd) as stream:
                display = stream.readline().strip()
            assert display, 'Xvfb display allocation failed'
            env['DISPLAY'] = ':' + display
            editor = subprocess.Popen([str(binary), '--new-window', str(project)], env=env, stdout=log, stderr=log)
            connection = None

            def wait_for(predicate, description, seconds=60):
                deadline = time.monotonic() + seconds
                while time.monotonic() < deadline:
                    assert editor.poll() is None, 'Editor exited unexpectedly'
                    result = predicate()
                    if result:
                        return result
                    time.sleep(0.05)
                raise AssertionError(description)

            def call(method, **params):
                with socket.socket(socket.AF_UNIX) as client:
                    client.settimeout(10)
                    client.connect(str(connection))
                    client.sendall((json.dumps({'id': 1, 'method': method, 'params': params}) + '\n').encode())
                    data = b''
                    while not data.endswith(b'\n'):
                        chunk = client.recv(65536)
                        assert chunk, 'Control socket disconnected'
                        data += chunk
                response = json.loads(data)
                assert 'error' not in response, response.get('error')
                return response['result']

            try:
                connection = wait_for(lambda: next((runtime / 'lumen-edit').glob('ctl-*.sock'), None), 'Control socket missing')
                wait_for(lambda: call('state').get('workspace'), 'Project not ready')
                window = subprocess.check_output(['xdotool', 'search', '--sync', '--pid', str(editor.pid)], env=env, text=True).splitlines()[0]
                subprocess.run(['xdotool', 'windowfocus', '--sync', window], env=env, check=True)
                for filename, original, line, column in fixtures:
                    source = original.replace('   \n', '    \n', 1)
                    call('set_cursor', path=filename, line=line, column=column)
                    # Allow focus + state observation to complete before issuing manually.
                    time.sleep(0.2)
                    baseline = call('state')['inline_completion']['accepted']
                    subprocess.run(['xdotool', 'key', '--clearmodifiers', 'space'], env=env, check=True)
                    def visible():
                        state = call('state')
                        stats = state['inline_completion']
                        if not stats['busy'] and stats['errors']:
                            raise AssertionError(stats['status'])
                        return state.get('active', {}).get('inline_completion_visible')
                    wait_for(visible, 'Provider did not produce a visible ghost')
                    assert call('read_file', path=filename) == source, 'Ghost modified the rope'
                    image = f'/tmp/lumen-inline-{Path(filename).suffix[1:]}.png'
                    call('screenshot', path=image)
                    subprocess.run(['xdotool', 'key', '--clearmodifiers', 'Tab'], env=env, check=True)
                    wait_for(lambda: call('state')['inline_completion']['accepted'] == baseline + 1, 'Tab did not accept')
                    assert call('read_file', path=filename) != source
                    call('run_command', command='undo')
                    assert call('read_file', path=filename) == source, 'Undo did not revert atomically'
                    print(filename, json.dumps(call('state')['inline_completion']), image)
                    call('close_file', path=filename, force=True)
                print('PASS: automatic debounce from real keyboard input, OpenRouter, Rust/Python fallback, native ghost, Tab and atomic Undo; isolated config')
            except Exception:
                log.flush()
                print((root / 'editor.log').read_text()[-4000:], file=__import__('sys').stderr)
                raise
            finally:
                editor.terminate()
                try:
                    editor.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    editor.kill()
                    editor.wait()
                xvfb.terminate()
                xvfb.wait(timeout=10)


if __name__ == '__main__':
    main()
