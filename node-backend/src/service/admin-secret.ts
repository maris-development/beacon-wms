import { compareSync } from "bcryptjs";
import { timingSafeEqual } from "crypto";
import logger from "./logger";

/// A bcrypt hash: $2<revision>$<cost>$<22 char salt><31 char digest>.
/// bcryptjs accepts the revisions a, b and y.
const BCRYPT_PATTERN = /^\$2[aby]?\$\d{2}\$[./A-Za-z0-9]{53}$/;

let plaintextWarningSent = false;

export class AdminSecret {

    /// Read ADMIN_SECRET. It holds a bcrypt hash, or a plaintext secret.
    static get(): string {
        return process.env.ADMIN_SECRET || "";
    }

    static isSet(): boolean {
        return AdminSecret.get().trim().length > 0;
    }

    static isHash(secret: string): boolean {
        return BCRYPT_PATTERN.test(secret);
    }

    /// Compare a token with ADMIN_SECRET. Plaintext secrets still pass, with a warning.
    static verify(token: string): boolean {
        const secret = AdminSecret.get();

        if (!token || secret.trim().length === 0) {
            return false;
        }

        if (AdminSecret.isHash(secret)) {
            return compareSync(token, secret);
        }

        AdminSecret.warnPlaintext();

        return AdminSecret.equalsConstantTime(token, secret);
    }

    /// Log the plaintext warning one time per process.
    private static warnPlaintext() {
        if (plaintextWarningSent) {
            return;
        }

        plaintextWarningSent = true;

        logger.warn(
            "ADMIN_SECRET holds a plaintext value. Replace it with a bcrypt hash. " +
            "Run: npx bcrypt \"<secret>\" 12"
        );
    }

    /// Report the secret format at startup, so an operator sees it before a request.
    static reportFormat() {
        if (!AdminSecret.isSet()) {
            logger.info("ADMIN_SECRET is empty. Admin endpoints stay closed.");

            return;
        }

        if (AdminSecret.isHash(AdminSecret.get())) {
            // logger.info("ADMIN_SECRET holds a bcrypt hash.");

            return;
        }

        AdminSecret.warnPlaintext();
    }

    /// Compare two strings of equal length in a constant time.
    private static equalsConstantTime(a: string, b: string): boolean {
        const left = Buffer.from(a, "utf8");
        const right = Buffer.from(b, "utf8");

        if (left.length !== right.length) {
            return false;
        }

        return timingSafeEqual(left, right);
    }
}
