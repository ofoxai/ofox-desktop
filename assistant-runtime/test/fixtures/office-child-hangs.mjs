// An Office reader child that starts and then never answers.
process.send({ ready: true });
setInterval(() => {}, 1000);
