const requiredEnv = ["GITHUB_TOKEN", "GITHUB_REPOSITORY", "RELEASE_ID", "VERSION"];

function fail(message) {
  throw new Error(message);
}

function assert(condition, message) {
  if (!condition) fail(message);
}

function findMacAssets(assets) {
  const dmg = assets.find((asset) => asset.name.endsWith(".dmg"));
  const updater = assets.find((asset) => asset.name.endsWith(".app.tar.gz"));
  const signature = updater
    ? assets.find((asset) => asset.name === `${updater.name}.sig`)
    : undefined;

  return { dmg, updater, signature };
}

function assertValidLatestJson(latestJson, assets) {
  const { dmg, updater, signature } = findMacAssets(assets);

  assert(dmg, "Missing macOS .dmg release asset");
  assert(updater, "Missing macOS updater .app.tar.gz release asset");
  assert(signature, `Missing signature asset for ${updater?.name ?? "macOS updater archive"}`);
  assert(latestJson && typeof latestJson === "object", "latest.json must be a JSON object");
  assert(latestJson.version, "latest.json is missing version");
  assert(latestJson.platforms && typeof latestJson.platforms === "object", "latest.json is missing platforms");

  for (const key of ["darwin-x86_64", "darwin-aarch64"]) {
    const platform = latestJson.platforms[key];
    assert(platform, `latest.json is missing ${key}`);
    assert(platform.url === updater.browser_download_url, `${key} must point to the universal .app.tar.gz asset`);
    assert(typeof platform.signature === "string" && platform.signature.trim().length > 0, `${key} signature is empty`);
    assert(!platform.url.endsWith(".dmg"), `${key} must not point to the .dmg installer`);
  }

  for (const [key, platform] of Object.entries(latestJson.platforms)) {
    assert(platform && typeof platform === "object", `${key} updater entry must be an object`);
    assert(typeof platform.url === "string" && platform.url.startsWith("https://"), `${key} updater URL must be HTTPS`);
    assert(typeof platform.signature === "string" && platform.signature.trim().length > 0, `${key} signature is empty`);
  }

  assert(
    latestJson.platforms["darwin-x86_64"].signature === latestJson.platforms["darwin-aarch64"].signature,
    "Universal macOS updater entries must use the same signature"
  );
}

function buildLatestJson(existingLatestJson, assets, version, notes, signatureText) {
  const { updater } = findMacAssets(assets);
  assert(updater, "Cannot build latest.json without a macOS .app.tar.gz asset");
  assert(signatureText.trim().length > 0, "Cannot build latest.json with an empty macOS signature");

  const latestJson = {
    version,
    notes,
    pub_date: new Date().toISOString(),
    platforms: {},
    ...(existingLatestJson && typeof existingLatestJson === "object" ? existingLatestJson : {}),
  };

  latestJson.version = version;
  latestJson.notes = latestJson.notes || notes;
  latestJson.pub_date = latestJson.pub_date || new Date().toISOString();
  latestJson.platforms = {
    ...(latestJson.platforms && typeof latestJson.platforms === "object" ? latestJson.platforms : {}),
    "darwin-x86_64": {
      signature: signatureText.trim(),
      url: updater.browser_download_url,
    },
    "darwin-aarch64": {
      signature: signatureText.trim(),
      url: updater.browser_download_url,
    },
  };

  return latestJson;
}

async function githubRequest(path, options = {}) {
  const token = process.env.GITHUB_TOKEN;
  const response = await fetch(`https://api.github.com${path}`, {
    ...options,
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${token}`,
      "X-GitHub-Api-Version": "2022-11-28",
      ...(options.headers || {}),
    },
  });

  if (!response.ok) {
    const body = await response.text();
    fail(`GitHub API request failed: ${response.status} ${response.statusText}\n${body}`);
  }

  if (response.status === 204) return null;
  return response.json();
}

async function downloadAsset(asset) {
  const response = await fetch(asset.url, {
    headers: {
      Accept: "application/octet-stream",
      Authorization: `Bearer ${process.env.GITHUB_TOKEN}`,
      "X-GitHub-Api-Version": "2022-11-28",
    },
  });

  if (!response.ok) {
    const body = await response.text();
    fail(`Failed to download ${asset.name}: ${response.status} ${response.statusText}\n${body}`);
  }

  return response.text();
}

async function uploadLatestJson(owner, repo, releaseId, latestJson, existingAsset) {
  if (existingAsset) {
    await githubRequest(`/repos/${owner}/${repo}/releases/assets/${existingAsset.id}`, {
      method: "DELETE",
    });
  }

  const body = JSON.stringify(latestJson, null, 2);
  const response = await fetch(
    `https://uploads.github.com/repos/${owner}/${repo}/releases/${releaseId}/assets?name=latest.json`,
    {
      method: "POST",
      headers: {
        Accept: "application/vnd.github+json",
        Authorization: `Bearer ${process.env.GITHUB_TOKEN}`,
        "Content-Type": "application/json",
        "Content-Length": Buffer.byteLength(body).toString(),
        "X-GitHub-Api-Version": "2022-11-28",
      },
      body,
    }
  );

  if (!response.ok) {
    const responseBody = await response.text();
    fail(`Failed to upload latest.json: ${response.status} ${response.statusText}\n${responseBody}`);
  }
}

async function syncRelease() {
  for (const name of requiredEnv) {
    assert(process.env[name], `Missing required env var: ${name}`);
  }

  const [owner, repo] = process.env.GITHUB_REPOSITORY.split("/");
  const releaseId = process.env.RELEASE_ID;
  const version = process.env.VERSION;
  const notes = process.env.RELEASE_NOTES || "See the release notes for details.";

  const assets = await githubRequest(`/repos/${owner}/${repo}/releases/${releaseId}/assets?per_page=100`);
  const latestAsset = assets.find((asset) => asset.name === "latest.json");
  const { signature } = findMacAssets(assets);

  assert(signature, "Cannot update latest.json without the macOS updater signature asset");

  let existingLatestJson = {};
  if (latestAsset) {
    const latestText = await downloadAsset(latestAsset);
    existingLatestJson = JSON.parse(latestText);
  }

  const signatureText = await downloadAsset(signature);
  const latestJson = buildLatestJson(existingLatestJson, assets, version, notes, signatureText);
  assertValidLatestJson(latestJson, assets);

  await uploadLatestJson(owner, repo, releaseId, latestJson, latestAsset);
  console.log("Validated and uploaded latest.json with universal macOS updater entries.");
}

function selfTest() {
  const assets = [
    {
      id: 1,
      name: "Telegram Drive_1.3.2_universal.dmg",
      browser_download_url: "https://github.com/example/repo/releases/download/v1.3.2/Telegram%20Drive_1.3.2_universal.dmg",
    },
    {
      id: 2,
      name: "Telegram Drive.app.tar.gz",
      browser_download_url: "https://github.com/example/repo/releases/download/v1.3.2/Telegram%20Drive.app.tar.gz",
    },
    {
      id: 3,
      name: "Telegram Drive.app.tar.gz.sig",
      browser_download_url: "https://github.com/example/repo/releases/download/v1.3.2/Telegram%20Drive.app.tar.gz.sig",
    },
  ];

  const latestJson = buildLatestJson({}, assets, "1.3.2", "Test release", "trusted-signature");
  assertValidLatestJson(latestJson, assets);

  const tampered = structuredClone(latestJson);
  tampered.platforms["darwin-aarch64"].url = assets[0].browser_download_url;
  try {
    assertValidLatestJson(tampered, assets);
    fail("Tampered updater manifest was accepted");
  } catch (error) {
    assert(error.message.includes("must point to the universal .app.tar.gz asset"), "Unexpected tamper failure");
  }

  const missingSignatureAssets = assets.filter((asset) => !asset.name.endsWith(".sig"));
  try {
    assertValidLatestJson(latestJson, missingSignatureAssets);
    fail("Missing signature asset was accepted");
  } catch (error) {
    assert(error.message.includes("Missing signature asset"), "Unexpected missing signature failure");
  }

  console.log("macOS updater release self-test passed.");
}

const mode = process.argv[2] || "sync";

if (mode === "self-test") {
  selfTest();
} else if (mode === "sync") {
  await syncRelease();
} else {
  fail(`Unknown mode: ${mode}`);
}
