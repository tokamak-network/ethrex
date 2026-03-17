/**
 * GitHub PR Helper — Create branches, commit files, and open PRs
 * via the GitHub REST API. (Next.js / Vercel compatible)
 */

const REPO_OWNER = process.env.METADATA_REPO_OWNER || "tokamak-network";
const REPO_NAME = process.env.METADATA_REPO_NAME || "tokamak-rollup-metadata-repository";
const GITHUB_TOKEN = process.env.GITHUB_BOT_TOKEN || process.env.GITHUB_TOKEN || null;

const HEADERS: Record<string, string> = {
  Accept: "application/vnd.github.v3+json",
  "Content-Type": "application/json",
  "User-Agent": "tokamak-platform-registry",
};

function authHeaders() {
  if (!GITHUB_TOKEN) throw new Error("GITHUB_BOT_TOKEN is required");
  return { ...HEADERS, Authorization: `Bearer ${GITHUB_TOKEN}` };
}

const API = `https://api.github.com/repos/${REPO_OWNER}/${REPO_NAME}`;

export async function getBranchSHA(branch = "main") {
  const res = await fetch(`${API}/git/ref/heads/${branch}`, {
    headers: authHeaders(),
  });
  if (!res.ok) throw new Error(`getBranchSHA: ${res.status} ${await res.text()}`);
  const data = await res.json();
  return data.object.sha as string;
}

export async function createBranch(branchName: string, baseSha: string) {
  const res = await fetch(`${API}/git/refs`, {
    method: "POST",
    headers: authHeaders(),
    body: JSON.stringify({ ref: `refs/heads/${branchName}`, sha: baseSha }),
  });
  if (res.status === 422) return; // Branch already exists
  if (!res.ok) throw new Error(`createBranch: ${res.status} ${await res.text()}`);
}

export async function createOrUpdateFile(filePath: string, content: string, branch: string, commitMessage: string) {
  let existingSHA: string | null = null;
  const checkRes = await fetch(`${API}/contents/${filePath}?ref=${branch}`, { headers: authHeaders() });
  if (checkRes.ok) {
    const existing = await checkRes.json();
    existingSHA = existing.sha;
  }

  const body: Record<string, unknown> = {
    message: commitMessage,
    content: Buffer.from(content).toString("base64"),
    branch,
  };
  if (existingSHA) body.sha = existingSHA;

  const res = await fetch(`${API}/contents/${filePath}`, {
    method: "PUT",
    headers: authHeaders(),
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`createOrUpdateFile: ${res.status} ${await res.text()}`);
  const data = await res.json();
  return data.commit.sha as string;
}

export async function createPullRequest(title: string, body: string, head: string, base = "main") {
  const res = await fetch(`${API}/pulls`, {
    method: "POST",
    headers: authHeaders(),
    body: JSON.stringify({ title, body, head, base }),
  });
  if (!res.ok) throw new Error(`createPullRequest: ${res.status} ${await res.text()}`);
  const data = await res.json();

  // Enable auto-merge (squash)
  try {
    await fetch("https://api.github.com/graphql", {
      method: "POST",
      headers: authHeaders(),
      body: JSON.stringify({
        query: `mutation {
          enablePullRequestAutoMerge(input: { pullRequestId: "${data.node_id}", mergeMethod: SQUASH }) {
            pullRequest { autoMergeRequest { enabledAt } }
          }
        }`,
      }),
    });
  } catch { /* ignore */ }

  return { prUrl: data.html_url as string, prNumber: data.number as number };
}

export async function getPullRequest(prNumber: number) {
  const res = await fetch(`${API}/pulls/${prNumber}`, { headers: authHeaders() });
  if (!res.ok) throw new Error(`getPullRequest: ${res.status} ${await res.text()}`);
  return res.json();
}

export async function findOpenPR(filePath: string) {
  const res = await fetch(`${API}/pulls?state=open&per_page=50`, { headers: authHeaders() });
  if (!res.ok) return null;
  const prs = await res.json();
  const filename = filePath.split("/").pop()!.replace(".json", "");
  for (const pr of prs) {
    if (!pr.title.includes(filename)) continue;
    try {
      const filesRes = await fetch(`${API}/pulls/${pr.number}/files`, { headers: authHeaders() });
      if (!filesRes.ok) continue;
      const files = await filesRes.json();
      if (files.some((f: { filename: string }) => f.filename === filePath)) {
        return { prUrl: pr.html_url as string, prNumber: pr.number as number, headBranch: pr.head.ref as string };
      }
    } catch { /* skip */ }
  }
  return null;
}

export async function updatePullRequest(prNumber: number, updates: { title?: string; body?: string }) {
  const filtered: Record<string, string> = {};
  if (updates.title) filtered.title = updates.title;
  if (updates.body) filtered.body = updates.body;
  if (Object.keys(filtered).length === 0) return;

  const res = await fetch(`${API}/pulls/${prNumber}`, {
    method: "PATCH",
    headers: authHeaders(),
    body: JSON.stringify(filtered),
  });
  if (!res.ok) throw new Error(`updatePullRequest: ${res.status} ${await res.text()}`);
  return res.json();
}

export async function getFileContent(filePath: string, branch = "main") {
  const res = await fetch(`${API}/contents/${filePath}?ref=${branch}`, { headers: authHeaders() });
  if (!res.ok) return null;
  const data = await res.json();
  if (!data.content) return null;
  const decoded = Buffer.from(data.content, "base64").toString("utf-8");
  try {
    return JSON.parse(decoded);
  } catch {
    return null;
  }
}
