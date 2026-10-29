self.onmessage = ({ data }) => {
  if (data?.type !== 'load') return;
  self.onmessage = null;
  import(data.url).catch((error) => {
    self.postMessage({ type: 'error', message: error instanceof Error ? error.message : String(error) });
  });
};
