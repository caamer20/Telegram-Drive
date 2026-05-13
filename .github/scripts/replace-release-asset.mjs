const fs = await import('node:fs/promises');
const path = await import('node:path');

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function githubRequest(url, options = {}) {
  const response = await fetch(url, {
    ...options,
    headers: {
      Accept: 'application/vnd.github+json',
      Authorization: `Bearer ${process.env.GITHUB_TOKEN}`,
      'X-GitHub-Api-Version': '2022-11-28',
      ...(options.headers || {}),
    },
  });

  if (!response.ok) {
    const body = await response.text();
    throw new Error(`GitHub API request failed: ${response.status} ${response.statusText}\n${body}`);
  }

  if (response.status === 204) return null;
  return response.json();
}

async function main() {
  const token = process.env.GITHUB_TOKEN;
  const repo = process.env.GITHUB_REPOSITORY;
  const releaseId = process.env.RELEASE_ID;
  const assetPath = process.env.ASSET_PATH;

  assert(token, 'Missing GITHUB_TOKEN');
  assert(repo, 'Missing GITHUB_REPOSITORY');
  assert(releaseId, 'Missing RELEASE_ID');
  assert(assetPath, 'Missing ASSET_PATH');

  const [owner, name] = repo.split('/');
  const assetName = path.basename(assetPath);
  const releaseAssets = await githubRequest(`https://api.github.com/repos/${owner}/${name}/releases/${releaseId}/assets?per_page=100`);
  const existing = releaseAssets.find((asset) => asset.name === assetName);

  if (existing) {
    await githubRequest(`https://api.github.com/repos/${owner}/${name}/releases/assets/${existing.id}`, {
      method: 'DELETE',
    });
  }

  const fileBuffer = await fs.readFile(assetPath);
  const uploadUrl = `https://uploads.github.com/repos/${owner}/${name}/releases/${releaseId}/assets?name=${encodeURIComponent(assetName)}`;
  const uploadResponse = await fetch(uploadUrl, {
    method: 'POST',
    headers: {
      Accept: 'application/vnd.github+json',
      Authorization: `Bearer ${token}`,
      'Content-Type': 'application/octet-stream',
      'Content-Length': String(fileBuffer.byteLength),
      'X-GitHub-Api-Version': '2022-11-28',
    },
    body: fileBuffer,
  });

  if (!uploadResponse.ok) {
    const body = await uploadResponse.text();
    throw new Error(`Asset upload failed: ${uploadResponse.status} ${uploadResponse.statusText}\n${body}`);
  }

  console.log(`Uploaded release asset: ${assetName}`);
}

await main();
