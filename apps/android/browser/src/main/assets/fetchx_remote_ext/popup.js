"use strict";

const button = document.getElementById("connect");
const listDownloadsButton = document.getElementById("list-downloads");
const form = document.getElementById("download-form");
const urlInput = document.getElementById("download-url");
const result = document.getElementById("result");

async function callRpc(request) {
  try {
    const response = await browser.runtime.sendMessage({ type: "fetchx-rpc", request });
    if (typeof response === "string") return JSON.parse(response);
    return response ?? { error: { message: "FetchX did not return an RPC response." } };
  } catch {
    result.textContent = "FetchX Android RPC is unavailable.";
    return null;
  }
}

button.addEventListener("click", async () => {
  button.disabled = true;
  result.textContent = "Connecting…";
  const response = await callRpc({
    jsonrpc: "2.0",
    id: crypto.randomUUID(),
    method: "system.discover",
  });
  if (response?.result?.protocol_version === "fetchx.rpc.v1") {
    result.textContent = `Connected · ${response.result.protocol_version}`;
  } else if (response) {
    result.textContent = "FetchX returned an incompatible RPC response.";
  }
  button.disabled = false;
});

listDownloadsButton.addEventListener("click", async () => {
  listDownloadsButton.disabled = true;
  result.textContent = "Loading downloads…";
  const response = await callRpc({
    jsonrpc: "2.0",
    id: crypto.randomUUID(),
    method: "download.list",
  });
  if (Array.isArray(response?.result?.downloads)) {
    result.textContent = `${response.result.downloads.length} download(s)`;
  } else if (response?.error) {
    result.textContent = "FetchX could not list downloads.";
  } else if (response) {
    result.textContent = "FetchX returned an incompatible RPC response.";
  }
  listDownloadsButton.disabled = false;
});

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  const url = urlInput.value.trim();
  let parsed;
  try {
    parsed = new URL(url);
  } catch {
    result.textContent = "Enter a valid HTTP or HTTPS URL.";
    return;
  }
  if ((parsed.protocol !== "http:" && parsed.protocol !== "https:")
      || parsed.username || parsed.password || url.length > 16384) {
    result.textContent = "Only credential-free HTTP or HTTPS URLs are accepted.";
    return;
  }

  const submit = form.querySelector("button");
  submit.disabled = true;
  result.textContent = "Submitting download…";
  const response = await callRpc({
    jsonrpc: "2.0",
    id: crypto.randomUUID(),
    method: "download.create",
    params: { url },
  });
  if (response?.result?.id) {
    result.textContent = "Download added to FetchX.";
    urlInput.value = "";
  } else if (response?.error) {
    result.textContent = "FetchX rejected the download request.";
  }
  submit.disabled = false;
});
