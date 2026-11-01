
import { spawn } from 'node:child_process';
import { writeFileSync, renameSync } from 'node:fs';
const [role, ready, nonce] = process.argv.slice(2);
const publish = data => {
  writeFileSync(ready + '.partial', JSON.stringify({ nonce, ...data }));
  renameSync(ready + '.partial', ready);
};
setInterval(() => {}, 1000);
if (role === 'sentinel') publish({ role, pid: process.pid });
else if (role === 'child') process.send({ nonce, pid: process.pid, parent: process.ppid });
else if (role === 'parent') {
  const child = spawn(process.execPath, [import.meta.filename, 'child', ready, nonce], {
    windowsHide: true, stdio: ['ignore', 'ignore', 'ignore', 'ipc']
  });
  child.once('error', () => process.exit(2));
  child.once('message', message => {
    if (message.nonce !== nonce || message.pid !== child.pid || message.parent !== process.pid)
      process.exit(3);
    publish({ role, parent: process.pid, child: child.pid });
  });
} else process.exit(4);
