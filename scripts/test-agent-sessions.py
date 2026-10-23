#!/usr/bin/env python3
"""Model-free Linux GUI acceptance test. Requires Xvfb; all state is temporary.

Usage: python3 scripts/test-agent-sessions.py [target/debug/lumen-edit]
The editor and a deterministic Pi RPC fixture run as separate processes. The
fixture never invokes a provider, shell tool, network service or real Pi config.
"""
import json
import datetime
import uuid
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time

FAKE_PI = r'''#!/usr/bin/env python3
import json,os,pathlib,sys,uuid
args=sys.argv[1:]
def arg(k): return args[args.index(k)+1] if k in args else None
folder=pathlib.Path.home()/'.pi/agent/sessions'/('--'+os.getcwd().lstrip('/').replace('/','-')+'--')
folder.mkdir(parents=True,exist_ok=True)
source=arg('--session') or arg('--fork')
path=pathlib.Path(source) if arg('--session') else folder/(str(uuid.uuid4())+'.jsonl')
if arg('--fork'): path.write_text(pathlib.Path(source).read_text())
if not path.exists(): path.write_text(json.dumps({'type':'session','cwd':os.getcwd(),'id':path.stem})+'\n')
name=arg('--name') or 'Mock session'
messages=[]
for line in path.read_text().splitlines():
    entry=json.loads(line)
    if entry['type']=='message': messages.append(entry['message'])
    if entry['type']=='session_info': name=entry.get('name') or name
if arg('--name'):
    name=arg('--name')
    with path.open('a') as f:f.write(json.dumps({'type':'session_info','name':name})+'\n')
streaming=False
log=pathlib.Path(os.environ['TEST_RPC_LOG'])/(os.environ['LUMEN_AGENT_ID']+'.jsonl')
def emit(event): print(json.dumps(event),flush=True)
def append(entry):
    with path.open('a') as f:f.write(json.dumps(entry)+'\n')
def reply(c,data=None): emit({'type':'response','id':c.get('id'),'command':c['type'],'success':True,'data':data or {}})
def message(role,text,error=False):
    m={'role':role,'content':[{'type':'text','text':text}]}
    if error:m.update(stopReason='error',errorMessage='Fixture provider error')
    messages.append(m);append({'type':'message','message':m})
    emit({'type':'message_start','message':m});emit({'type':'message_end','message':m})
def finish(text,error=False):
    global streaming
    message('assistant',text,error);streaming=False;emit({'type':'agent_settled','aborted':False})
for line in sys.stdin:
    c=json.loads(line)
    with log.open('a') as f:f.write(json.dumps({'pid':os.getpid(),'path':str(path),'command':c,'args':args})+'\n')
    method=c['type']
    if method=='get_state': reply(c,{'sessionFile':str(path),'sessionId':path.stem,'sessionName':name,'isStreaming':streaming,'isCompacting':False,'thinkingLevel':'off'})
    elif method=='get_messages':reply(c,{'messages':messages})
    elif method=='get_session_stats':reply(c,{'sessionFile':str(path),'tokens':{'total':1200},'cost':0.001,'contextUsage':{'percent':2,'tokens':1200,'contextWindow':60000}})
    elif method=='get_available_models':reply(c,{'models':[{'id':'test','name':'Fixture','provider':'mock'}]})
    elif method=='get_available_thinking_levels':reply(c,{'levels':['off']})
    elif method=='get_commands':reply(c,{'commands':[]})
    elif method=='set_session_name':
        name=c['name'];append({'type':'session_info','name':name});reply(c);emit({'type':'session_info_changed','name':name})
    elif method=='clear_queue':reply(c,{'steering':[],'followUp':[]});emit({'type':'queue_update','steering':[],'followUp':[]})
    elif method=='abort':streaming=False;reply(c);emit({'type':'agent_settled','aborted':True})
    elif method=='prompt':
        reply(c,{'disposition':'started'});streaming=True;emit({'type':'agent_start'});message('user',c['message'])
        if c['message']=='hold':continue
        if c['message']=='confirm':
            emit({'type':'extension_ui_request','id':'permission','method':'confirm','title':'Confirm fixture','message':'Long command\n'*300,'timeout':300})
            continue
        finish('Result: '+c['message'],c['message']=='fail')
    elif method=='follow_up':
        reply(c,{'disposition':'queued'})
        if streaming:emit({'type':'queue_update','steering':[],'followUp':[c['message']]})
        else:
            streaming=True;emit({'type':'agent_start'});message('user',c['message']);finish('Follow-up result')
    elif method=='extension_ui_response':finish('Confirmed')
    else:reply(c)
'''


def wait_for(fn, message, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = fn()
        if value:
            return value
        time.sleep(0.08)
    raise AssertionError(message)


def main():
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/lumen-edit').resolve()
    with tempfile.TemporaryDirectory(prefix='lumen-agent-acceptance-') as temporary:
        root = Path(temporary)
        home, config, runtime, logs, project = [root / name for name in ('home', 'config', 'runtime', 'logs', 'project')]
        for folder in (home, config / 'lumen-edit', runtime, logs, project):
            folder.mkdir(parents=True, exist_ok=True)
        runtime.chmod(0o700)
        fake = root / 'pi-fixture'
        fake.write_text(FAKE_PI)
        fake.chmod(0o755)
        (config / 'lumen-edit/config.toml').write_text(
            f'language = "en"\npi_command = {json.dumps(os.environ.get('LUMEN_TEST_REAL_PI', str(fake)))}\n'
            'agent_sandbox = false\nagent_mode = ""\nagent_model = ""\npulse = false\n')
        env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(config), XDG_STATE_HOME=str(root / 'state'),
                   XDG_RUNTIME_DIR=str(runtime), XDG_CACHE_HOME=str(root / 'cache'), TEST_RPC_LOG=str(logs), SHELL='/bin/sh')
        env.pop('WAYLAND_DISPLAY', None)
        process = None
        connection = None
        log_handle = (root / 'editor.log').open('w')
        # Let Xvfb atomically allocate a display. Concurrent xvfb-run -a wrappers
        # can choose the same display while one instance is restarting.
        display_read, display_write = os.pipe()
        display_server = subprocess.Popen(['Xvfb', '-displayfd', str(display_write), '-screen', '0', '1440x1000x24', '-nolisten', 'tcp', '-ac'],
                                          pass_fds=(display_write,), stdout=log_handle, stderr=log_handle)
        os.close(display_write)
        with os.fdopen(display_read) as display_pipe:
            display = display_pipe.readline().strip()
        assert display, 'Xvfb did not allocate a display'
        env['DISPLAY'] = ':' + display

        def launch():
            nonlocal process, connection
            process = subprocess.Popen([str(binary), '--new-window', str(project)],
                                       env=env, stdout=log_handle, stderr=log_handle)
            connection = wait_for(lambda: next((runtime / 'lumen-edit').glob('ctl-*.sock'), None), 'GUI control socket missing', 30)
            wait_for(lambda: call('state').get('workspace'), 'Project not loaded')

        def call(method, **params):
            with socket.socket(socket.AF_UNIX) as client:
                client.settimeout(15)
                client.connect(str(connection))
                client.sendall((json.dumps({'id': 1, 'method': method, 'params': params}) + '\n').encode())
                data = b''
                while not data.endswith(b'\n'):
                    data += client.recv(65536)
                response = json.loads(data)
            if 'error' in response:
                raise AssertionError(response['error'])
            return response['result']

        def agent(action, **params):
            return call('agent', action=action, **params)

        def mcp_agent(caller, arguments, granular=True):
            command = [str(binary), 'mcp'] + (['--granular'] if granular else [])
            request = {'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call', 'params': {'name': 'agent', 'arguments': arguments}}
            response = subprocess.run(command, input=json.dumps(request) + '\n', text=True, capture_output=True, timeout=20,
                                      env=dict(env, LUMEN_SOCKET=str(connection), LUMEN_AGENT_ID=str(caller), LUMEN_MCP_NO_LAUNCH='1'), check=True)
            result = json.loads(response.stdout)['result']
            assert not result.get('isError'), result
            return json.loads(result['content'][0]['text'])

        def live(): return agent('list')
        def lookup(identity): return next((a for a in live() if a['id'] == identity), None)
        def records(identity):
            path = logs / f'{identity}.jsonl'
            return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
        def result_count(identity):
            return sum(r['command']['type'] in ('prompt', 'follow_up') and r['command'].get('message', '').startswith('Worker ') for r in records(identity))
        def shutdown():
            nonlocal process
            if process and process.poll() is None:
                try: call('quit')
                except (OSError, AssertionError): process.terminate()
                try: process.wait(timeout=10)
                except subprocess.TimeoutExpired: process.kill(); process.wait()
            process = None

        try:
            launch()
            if os.environ.get('LUMEN_TEST_REAL_PI'):
                real_one = agent('create', name='RPC smoke one')['id']
                real_two = agent('create', name='RPC smoke two')['id']
                wait_for(lambda: len(live()) == 2 and all(a['ready'] and a['session'] for a in live()), 'Installed Pi did not become ready', 30)
                assert len({a['session'] for a in live()}) == 2
                assert all(a['running'] and not a['busy'] for a in live())
                agent('rename', agent=real_one, name='Real Pi renamed')
                wait_for(lambda: lookup(real_one)['name'] == 'π Real Pi renamed', 'Installed Pi rename failed')
                synthetic = root / 'synthetic-session.jsonl'
                stamp = datetime.datetime.now(datetime.timezone.utc).isoformat()
                synthetic.write_text('\n'.join(json.dumps(entry) for entry in [
                    {'type': 'session', 'version': 3, 'id': str(uuid.uuid4()), 'timestamp': stamp, 'cwd': str(project)},
                    {'type': 'message', 'id': 'fixture-user', 'parentId': None, 'timestamp': stamp,
                     'message': {'role': 'user', 'content': [{'type': 'text', 'text': 'Synthetic history for session management smoke test'}], 'timestamp': int(time.time() * 1000)}}
                ]) + '\n')
                saved = agent('switch', text=str(synthetic))['id']
                wait_for(lambda: lookup(saved)['ready'], 'Installed Pi could not load synthetic history')
                assert agent('transcript', agent=saved)['items'][0].endswith('Synthetic history for session management smoke test')
                clone = agent('clone', agent=saved)['id']
                wait_for(lambda: lookup(clone)['ready'] and lookup(clone)['session'], 'Installed Pi clone failed')
                assert lookup(clone)['session'] != str(synthetic)
                agent('restart', agent=saved)
                wait_for(lambda: lookup(saved)['ready'], 'Installed Pi exact restart failed')
                assert lookup(saved)['session'] == str(synthetic)
                shutdown()
                launch()
                wait_for(lambda: len(live()) == 4 and all(a['ready'] for a in live()), 'Installed-Pi sessions did not restore')
                assert any(a['name'] == 'π Real Pi renamed' for a in live())
                agent('close', agent=live()[-1]['id'])
                assert len(live()) == 3
                print('PASS: installed Pi RPC: independent sessions, state/stats, rename, history load, clone, exact restart, empty/saved session restoration and close; no prompts sent')
                return
            one = agent('create', name='Independent A')['id']
            two = agent('create', name='Independent B')['id']
            lead = agent('create', name='Coordinator', role='coordinator')['id']
            worker = mcp_agent(lead, {'action': 'create', 'name': 'Parser worker', 'coordinator': lead})['id']
            assert len(mcp_agent(lead, {'action': 'list'}, granular=False)) == 4
            wait_for(lambda: len(live()) == 4 and all(a['session'] for a in live()), 'Four distinct sessions failed to open')
            assert len({a['session'] for a in live()}) == 4
            path_one = lookup(one)['session']
            assert agent('switch', text=path_one)['id'] == one
            assert len(live()) == 4, 'Opening a live session must focus it, not duplicate it'
            agent('restart', agent=one)
            wait_for(lambda: len({r['pid'] for r in records(one)}) == 2, 'Restart did not replace process')
            wait_for(lambda: lookup(one)['ready'], 'Restart did not finish loading history')
            assert lookup(one)['session'] == path_one, 'Restart selected another session'
            time.sleep(3.4)
            assert lookup(one)['running'] and lookup(one)['exited'] is None, 'Stale exit event killed replacement'
            # A background coordinator must not default to the active GUI session.
            agent('focus', agent=two)
            assert agent('transcript', _caller_agent=lead)['id'] == lead
            try:
                agent('create', _caller_agent=worker, name='Illegal nested worker')
                raise AssertionError('Worker unexpectedly delegated')
            except AssertionError as error:
                assert 'only coordinators' in str(error)
            try:
                mcp_agent(worker, {'action': 'create', 'name': 'Nested worker'})
                raise AssertionError('MCP worker unexpectedly delegated')
            except AssertionError as error:
                assert 'only coordinators' in str(error)
            agent('focus', agent=two)
            agent('dispatch', agent=worker, text='Review parser only')
            wait_for(lambda: lookup(worker)['task']['delivered'], 'Worker result was not delivered')
            assert result_count(lead) == 1
            assert result_count(two) == 0, 'Worker result leaked to active independent session'
            time.sleep(0.2)
            assert result_count(lead) == 1, 'Result delivered more than once'
            agent('dispatch', agent=worker, text='fail')
            wait_for(lambda: lookup(worker)['task']['status'] == 'failed' and lookup(worker)['task']['delivered'], 'Failure not reported')
            assert result_count(lead) == 2
            agent('dispatch', agent=worker, text='hold')
            wait_for(lambda: lookup(worker)['streaming'], 'Worker did not start')
            agent('group_stop', agent=lead)
            wait_for(lambda: not lookup(worker)['streaming'], 'Group stop did not stop worker')
            assert lookup(lead)['paused'] and lookup(worker)['paused']
            assert lookup(worker)['task']['status'] == 'cancelled'
            assert result_count(lead) == 2, 'Stopping a group must not wake its coordinator'
            agent('resume_group', agent=lead)
            wait_for(lambda: lookup(worker)['task']['delivered'], 'Explicit resume did not deliver held result')
            agent('rename', agent=one, name='Renamed independent')
            wait_for(lambda: lookup(one)['name'] == 'π Renamed independent', 'Rename failed')
            agent('clone', agent=one)
            wait_for(lambda: len(live()) == 5 and all(a['session'] for a in live()), 'Clone missing')
            assert len({a['session'] for a in live()}) == 5
            agent('prompt', agent=two, text='confirm')
            wait_for(lambda: lookup(two)['pending_confirmations'] == 1, 'Hidden request was not counted')
            wait_for(lambda: lookup(two)['pending_confirmations'] == 0, 'Expired request remained visible')
            agent('abort', agent=two)
            agent('manager')
            time.sleep(0.6)
            screenshot = root / 'manager.png'
            call('screenshot', path=str(screenshot))
            output = Path('/tmp/lumen-agent-manager.png')
            output.write_bytes(screenshot.read_bytes())
            # In-flight tasks are persisted, but must not be resubmitted after editor restart.
            agent('dispatch', agent=worker, text='hold')
            wait_for(lambda: lookup(worker)['streaming'], 'Second hold did not start')
            original_paths = {a['session'] for a in live()}
            shutdown()
            launch()
            wait_for(lambda: len(live()) == 5 and all(a['session'] for a in live()), 'Sessions not restored')
            assert {a['session'] for a in live()} == original_paths
            restored_worker = next(a for a in live() if a['role'] == 'worker')
            restored_lead = next(a for a in live() if a['role'] == 'coordinator')
            assert restored_worker['coordinator'] == restored_lead['id']
            assert restored_worker['task']['status'] == 'cancelled'
            assert not any(a['streaming'] for a in live()), 'Restoration replayed a model request'
            agent('focus', agent=restored_lead['id'])
            call('run_command', command='close_tab')
            assert lookup(restored_lead['id']) is None
            assert lookup(restored_worker['id'])['role'] == 'independent'
            assert lookup(restored_worker['id'])['coordinator'] is None
            agent('close', agent=restored_worker['id'])
            assert len(live()) == 3
            print('PASS: isolated GUI: independent sessions, duplicate prevention, groups, targeted results, failure/stop/resume, rename/clone, expiring hidden dialogs, exact restoration without prompt replay, close')
            print('Screenshot:', output)
        except Exception:
            log_handle.flush()
            print((root / 'editor.log').read_text()[-5000:], file=sys.stderr)
            raise
        finally:
            shutdown()
            display_server.terminate()
            display_server.wait(timeout=10)
            log_handle.close()


if __name__ == '__main__':
    main()
