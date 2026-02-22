const crypto = require("crypto");
const { getUserById } = require("../db/users");

// In-memory session store
const sessions = new Map();
const SESSION_TTL = 24 * 60 * 60 * 1000; // 24 hours

function createSession(userId) {
  const token = "ps_" + crypto.randomBytes(32).toString("hex");
  sessions.set(token, { userId, createdAt: Date.now() });
  return token;
}

function getSession(token) {
  const session = sessions.get(token);
  if (!session) return null;
  if (Date.now() - session.createdAt > SESSION_TTL) {
    sessions.delete(token);
    return null;
  }
  return session;
}

function destroySession(token) {
  sessions.delete(token);
}

// Middleware: require authentication
async function requireAuth(req, res, next) {
  try {
    const bearer = req.headers["authorization"]?.replace("Bearer ", "");
    if (!bearer) {
      return res.status(401).json({ error: "Authentication required" });
    }

    const session = getSession(bearer);
    if (!session) {
      return res.status(401).json({ error: "Invalid or expired session" });
    }

    const user = getUserById(session.userId);
    if (!user) {
      sessions.delete(bearer);
      return res.status(401).json({ error: "User not found" });
    }
    if (user.status !== "active") {
      return res.status(403).json({ error: "Account is suspended" });
    }

    req.user = user;
    req.sessionToken = bearer;
    next();
  } catch (e) {
    next(e);
  }
}

// Middleware: require admin role
async function requireAdmin(req, res, next) {
  try {
    const bearer = req.headers["authorization"]?.replace("Bearer ", "");
    if (!bearer) {
      return res.status(401).json({ error: "Authentication required" });
    }

    const session = getSession(bearer);
    if (!session) {
      return res.status(401).json({ error: "Invalid or expired session" });
    }

    const user = getUserById(session.userId);
    if (!user || user.role !== "admin") {
      return res.status(403).json({ error: "Admin access required" });
    }

    req.user = user;
    req.sessionToken = bearer;
    next();
  } catch (e) {
    next(e);
  }
}

module.exports = { createSession, destroySession, requireAuth, requireAdmin };
