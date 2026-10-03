# Prepended to fake_codex.py; exercises real package files and subprocess configuration.
import os
import pathlib
import json
import sys
import tomllib

fixture_base = pathlib.Path(__file__).parent
if 'plugin' in sys.argv:
    if (fixture_base / 'fail-install').exists():
        print('fixture installation refused', file=sys.stderr)
        sys.exit(7)
    print(json.dumps({'installed': []}))
    sys.exit(0)


def record_tools(request):
    home = pathlib.Path(os.environ['CODEX_HOME'])
    config = tomllib.loads((home / 'config.toml').read_text()) if (home / 'config.toml').exists() else {}
    overrides = [arg for arg in sys.argv if arg.startswith('mcp_servers=')]
    mcp = tomllib.loads(overrides[0])['mcp_servers'] if overrides else None
    record = {'method': request['method'], 'home': str(home), 'args': sys.argv[1:], 'mcp': mcp,
              'plugins': [key for key, value in config.get('plugins', {}).items() if value.get('enabled')],
              'pid': os.getpid()}
    with pathlib.Path('tool-launches.jsonl').open('a') as output:
        output.write(json.dumps(record) + '\n')
