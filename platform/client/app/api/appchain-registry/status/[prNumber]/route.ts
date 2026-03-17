import { NextRequest, NextResponse } from "next/server";
import { getPullRequest } from "@/lib/github-pr";

export async function GET(
  _req: NextRequest,
  { params }: { params: Promise<{ prNumber: string }> },
) {
  try {
    const { prNumber: prNumberStr } = await params;
    const prNumber = parseInt(prNumberStr);
    if (isNaN(prNumber)) {
      return NextResponse.json({ error: "Invalid PR number" }, { status: 400 });
    }

    const pr = await getPullRequest(prNumber);
    return NextResponse.json({
      prNumber: pr.number,
      state: pr.state,
      merged: pr.merged,
      mergeable: pr.mergeable,
      title: pr.title,
      htmlUrl: pr.html_url,
    });
  } catch (e) {
    return NextResponse.json({ error: (e as Error).message }, { status: 500 });
  }
}
