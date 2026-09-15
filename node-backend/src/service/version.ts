import { existsSync, readFileSync } from "fs";
import path from "path";
import logger from "./logger";

const GIT_DIR = process.env.GIT_DIR || "/repo/.git";
const REPO_URL = process.env.GIT_REPO_URL || "https://github.com/maris-development/beacon-wms";

export interface VersionInfo {
    commit: string;
    shortCommit: string;
    branch: string;
    builtAt: string;
    commitUrl: string;
    source: "build" | "git" | "unknown";
}

/**
 * Report the commit that the running code comes from.
 *
 * `GIT_COMMIT` wins, because the build bakes it into the image. Without it the
 * service reads the mounted `.git` directory, which holds the commit of the
 * working copy that built the image. The result is read once and kept.
 */
export class Version {
    private static info: VersionInfo | null = null;

    public static get = (): VersionInfo => {
        Version.info = Version.info ?? Version.read();

        return Version.info;
    }

    /// Write the version to the log, so a support question has an answer.
    public static report = (): void => {
        const info = Version.get();

        logger.info(`Version ${info.shortCommit} on branch ${info.branch}, source ${info.source}`);
    }

    private static read = (): VersionInfo => {
        const builtAt = process.env.BUILD_TIME || "";
        const commit = (process.env.GIT_COMMIT || "").trim();

        if (commit) {
            return Version.build(commit, (process.env.GIT_BRANCH || "").trim(), builtAt, "build");
        }

        const fromGit = Version.readGitDir();

        if (fromGit) {
            return Version.build(fromGit.commit, fromGit.branch, builtAt, "git");
        }

        return Version.build("", "", builtAt, "unknown");
    }

    private static build = (commit: string, branch: string, builtAt: string, source: VersionInfo["source"]): VersionInfo => {
        return {
            commit: commit || "unknown",
            shortCommit: commit ? commit.slice(0, 7) : "unknown",
            branch: branch || "unknown",
            builtAt: builtAt || "unknown",
            commitUrl: commit ? `${REPO_URL.replace(/\/+$/, "")}/tree/${commit}` : "",
            source,
        };
    }

    /// Resolve HEAD without the git binary. A clone keeps most refs in `packed-refs`.
    private static readGitDir = (): { commit: string, branch: string } | null => {
        try {
            if (!existsSync(GIT_DIR)) {
                return null;
            }

            const head = readFileSync(path.join(GIT_DIR, "HEAD"), "utf-8").trim();

            if (!head.startsWith("ref:")) {
                return { commit: head, branch: "detached" };
            }

            const ref = head.slice(4).trim();
            const branch = ref.replace(/^refs\/heads\//, "");
            const commit = Version.readRef(ref);

            return commit ? { commit, branch } : null;

        } catch (err) {
            logger.error(`Failed to read the git directory '${GIT_DIR}': ${err}`);

            return null;
        }
    }

    private static readRef = (ref: string): string | null => {
        const loose = path.join(GIT_DIR, ref);

        if (existsSync(loose)) {
            return readFileSync(loose, "utf-8").trim();
        }

        const packed = path.join(GIT_DIR, "packed-refs");

        if (!existsSync(packed)) {
            return null;
        }

        for (const line of readFileSync(packed, "utf-8").split("\n")) {
            const [commit, name] = line.trim().split(" ");

            if (name === ref && commit) {
                return commit;
            }
        }

        return null;
    }
}
