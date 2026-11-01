// An Office reader child that starts and then dies abruptly, as on running
// out of memory.
process.send({ ready: true });
process.once("message", () => process.kill(process.pid, "SIGKILL"));
