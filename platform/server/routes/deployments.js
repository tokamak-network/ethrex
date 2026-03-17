const express = require("express");
const router = express.Router();

const { requireAuth } = require("../middleware/auth");
const {
  createDeployment,
  getDeploymentsByUser,
  getDeploymentById,
  updateDeployment,
  deleteDeployment,
} = require("../db/deployments");
const { getProgramById, incrementUseCount } = require("../db/programs");

router.use(requireAuth);

// POST /api/deployments — create a new deployment record
router.post("/", (req, res) => {
  try {
    const { programId, name, chainId, rpcUrl, config } = req.body;
    if (!programId || !name) {
      return res.status(400).json({ error: "programId and name are required" });
    }
    if (typeof name !== 'string' || name.trim().length > 200) {
      return res.status(400).json({ error: "name must be a string of 200 characters or fewer" });
    }

    const program = getProgramById(programId);
    if (!program || program.status !== "active") {
      return res.status(404).json({ error: "Program not found or not active" });
    }

    const deployment = createDeployment({
      userId: req.user.id,
      programId,
      name: name.trim(),
      chainId: chainId || null,
      rpcUrl: rpcUrl || null,
      config: config || null,
    });

    incrementUseCount(programId);
    res.status(201).json({ deployment });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments — list my deployments
router.get("/", (req, res) => {
  try {
    const deployments = getDeploymentsByUser(req.user.id);
    res.json({ deployments });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments/:id — get deployment detail
router.get("/:id", (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }
    res.json({ deployment });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// PUT /api/deployments/:id — update deployment config
router.put("/:id", (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }
    // Prevent setting status to 'active' directly; use /activate endpoint instead
    const fields = { ...req.body };
    if (fields.status === 'active') {
      delete fields.status;
    }
    const updated = updateDeployment(req.params.id, fields);
    res.json({ deployment: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// DELETE /api/deployments/:id — remove deployment record
router.delete("/:id", (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }
    deleteDeployment(req.params.id);
    res.json({ ok: true });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/activate — mark as active (used by Desktop after successful deploy)
router.post("/:id/activate", (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }
    const updated = updateDeployment(req.params.id, { status: "active" });
    res.json({ deployment: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/push-metadata — push metadata to GitHub repo
router.post("/:id/push-metadata", async (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }
    if (deployment.status !== "active") {
      return res.status(400).json({ error: "Deployment must be active to push metadata" });
    }

    const { pushMetadataToRepo } = require("../lib/metadata-push");
    const result = await pushMetadataToRepo(deployment);
    res.json({ success: true, path: result.path });
  } catch (e) {
    console.error("[push-metadata]", e.message);
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/delete-metadata — delete metadata from GitHub repo
router.post("/:id/delete-metadata", async (req, res) => {
  try {
    const deployment = getDeploymentById(req.params.id);
    if (!deployment || deployment.user_id !== req.user.id) {
      return res.status(404).json({ error: "Deployment not found" });
    }

    const { deleteMetadataFromRepo } = require("../lib/metadata-push");
    const result = await deleteMetadataFromRepo(deployment);
    res.json({ success: true, deleted: result.deleted });
  } catch (e) {
    console.error("[delete-metadata]", e.message);
    res.status(500).json({ error: e.message });
  }
});

module.exports = router;
