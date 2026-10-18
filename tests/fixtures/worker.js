self.onmessage = (e) => {
  self.postMessage('pong:' + e.data);
};
