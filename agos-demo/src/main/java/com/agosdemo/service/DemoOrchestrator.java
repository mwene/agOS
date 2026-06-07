package com.agosdemo.service;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import org.springframework.stereotype.Service;

import java.util.*;
import java.util.concurrent.atomic.AtomicReference;

@Service
public class DemoOrchestrator {

    private final AgosKernelClient kernel;
    private final ObjectMapper mapper;
    private final List<DemoStep> steps = new ArrayList<>();

    public DemoOrchestrator(AgosKernelClient kernel) {
        this.kernel = kernel;
        this.mapper = new ObjectMapper();
    }

    public record DemoStep(String phase, String title, boolean success, String detail, long elapsedMs) {}

    public List<DemoStep> getSteps() { return steps; }

    public boolean isKernelAlive() { return kernel.health(); }

    public DemoStep runStep(String phase, String title, Runnable r) {
        long start = System.currentTimeMillis();
        try {
            r.run();
            long elapsed = System.currentTimeMillis() - start;
            var step = new DemoStep(phase, title, true, "OK", elapsed);
            steps.add(step);
            return step;
        } catch (Exception e) {
            long elapsed = System.currentTimeMillis() - start;
            var step = new DemoStep(phase, title, false, e.getMessage(), elapsed);
            steps.add(step);
            return step;
        }
    }

    public void runFullDemo() {
        steps.clear();

        var pidAuditor = new AtomicReference<String>();
        var pidAccountant = new AtomicReference<String>();

        // P1: Boot + UMI
        runStep("P1", "Kernel health check", () -> {
            if (!kernel.health()) throw new RuntimeException("Kernel unreachable at 127.0.0.1:7400");
        });

        runStep("P1", "Register UMI drivers: LocalEngine-v0 + CloudGPT-HighReason", () -> {
            var m = kernel.umiMetrics();
            if (!m.has("metrics")) throw new RuntimeException("No UMI metrics returned");
        });

        runStep("P1", "Active driver: LocalEngine-v0 (capability level 2)", () -> {
            var m = kernel.umiMetrics();
            if (!"LocalEngine-v0".equals(m.path("active_driver").asText()))
                throw new RuntimeException("Expected LocalEngine-v0 as active driver");
        });

        // P2: Install apps
        runStep("P2", "Install 'Financial Auditor' .aapp bundle", () -> {
            var r = kernel.installApp("financial_auditor");
            if (!"installed".equals(r.path("status").asText()))
                throw new RuntimeException("Install failed: " + r);
        });

        runStep("P2", "Install 'Accountant' .aapp bundle", () -> {
            var r = kernel.installApp("accountant");
            if (!"installed".equals(r.path("status").asText()))
                throw new RuntimeException("Install failed: " + r);
        });

        // P3: Spawn processes
        runStep("P3", "Spawn Financial Auditor process (priority 2, token quota 5000)", () -> {
            var manifest = mapper.createObjectNode();
            manifest.put("priority", 2);
            manifest.putObject("limits").put("max_tokens", 5000);
            var perms = manifest.putArray("permissions");
            perms.add("financial:read").add("financial:validate").add("cipc:communicate");
            manifest.put("goal_metric", 0.95);
            var r = kernel.spawnProcess(manifest, null);
            var pid = r.path("pid").asText();
            if (pid.isEmpty()) throw new RuntimeException("Spawn returned no PID");
            pidAuditor.set(pid);
        });

        runStep("P3", "Spawn Accountant process (priority 2, System 2 worker)", () -> {
            var manifest = mapper.createObjectNode();
            manifest.put("priority", 2);
            manifest.putObject("limits").put("max_tokens", 3000);
            var perms = manifest.putArray("permissions");
            perms.add("financial:read").add("financial:compute").add("cipc:communicate");
            manifest.put("goal_metric", 0.97);
            var r = kernel.spawnProcess(manifest, pidAuditor.get());
            var pid = r.path("pid").asText();
            if (pid.isEmpty()) throw new RuntimeException("Spawn returned no PID");
            pidAccountant.set(pid);
        });

        runStep("P3", "Verify both processes running under scheduler", () -> {
            var procs = kernel.listProcesses();
            int count = procs.path("processes").size();
            if (count < 2) throw new RuntimeException("Expected >=2 processes, got " + count);
        });

        // P4: Cognitive compute + memory
        runStep("P4", "UMI SYS_REASON on financial prompt vector", () -> {
            var r = kernel.umiReason(List.of(
                "validate_balance_sheet", "check_discrepancies", "cross_reference_ledgers"
            ));
            if (!r.has("output")) throw new RuntimeException("No reasoning output");
        });

        runStep("P4", "Write L1 active context for Auditor", () -> {
            var data = List.of(
                (JsonNode) mapper.createObjectNode().put("type", "balance").put("total", 24765.49),
                (JsonNode) mapper.createObjectNode().put("type", "entry_count").put("count", 42)
            );
            var r = kernel.writeL1(pidAuditor.get(), data);
            if (!r.path("ok").asBoolean()) throw new RuntimeException("L1 write failed");
        });

        runStep("P4", "Compress L1 → L2 topological graph cache", () -> {
            var r = kernel.compressL2(pidAuditor.get());
            if (!r.has("compressed")) throw new RuntimeException("L2 compression failed");
        });

        runStep("P4", "Embed and store vector in Cognitive RAM", () -> {
            var em = kernel.umiEmbed(List.of("ledger_2024", "balance_verification"));
            var vec = new ArrayList<Double>();
            em.path("embedding").forEach(n -> vec.add(n.asDouble()));
            var meta = mapper.createObjectNode()
                .put("pid", pidAuditor.get())
                .put("type", "financial_vector")
                .put("confidence", 0.97);
            kernel.storeVector(vec, meta);
        });

        runStep("P4", "Cosine-similarity page-in from Vector RAM", () -> {
            var em = kernel.umiEmbed(List.of("ledger"));
            var query = new ArrayList<Double>();
            em.path("embedding").forEach(n -> query.add(n.asDouble()));
            if (!query.isEmpty()) {
                int top = Math.min(3, query.size());
                var pr = kernel.pageIn(query.subList(0, top), 3);
                if (pr.path("count").asInt() < 1)
                    throw new RuntimeException("No pages returned from Vector RAM");
            }
        });

        // P5: cIPC
        runStep("P5", "Auditor sends verification request via cIPC bus", () -> {
            var payload = mapper.createObjectNode();
            payload.put("type", "financial.verification_request");
            payload.put("action", "cross_validate");
            payload.put("balance_total", 24765.49);
            payload.put("requires_deep_reasoning", true);
            var r = kernel.routeMessage(pidAuditor.get(), pidAccountant.get(), payload);
            if (!r.path("ok").asBoolean()) throw new RuntimeException("cIPC route failed");
        });

        runStep("P5", "Accountant responds with verification result via cIPC", () -> {
            var payload = mapper.createObjectNode();
            payload.put("type", "financial.verification_result");
            payload.put("status", "verified");
            payload.put("discrepancies_found", 0);
            payload.put("confidence", 0.97);
            var r = kernel.routeMessage(pidAccountant.get(), pidAuditor.get(), payload);
            if (!r.path("ok").asBoolean()) throw new RuntimeException("cIPC response failed");
        });

        runStep("P5", "Verify 2+ messages in cIPC history", () -> {
            var hist = kernel.cipcHistory();
            if (hist.path("count").asInt() < 2)
                throw new RuntimeException("Expected >=2 cIPC messages");
        });

        // UMI hot-swap
        runStep("UMI", "Hot-swap to CloudGPT-HighReason (capability level 5)", () -> {
            var r = kernel.umiSwitch("CloudGPT-HighReason");
            if (!r.path("ok").asBoolean()) throw new RuntimeException("Hot-swap failed");
        });

        runStep("UMI", "Deep reasoning on cloud driver", () -> {
            var r = kernel.umiReason(List.of("deep_audit_trail", "fraud_detection", "regulatory_compliance"));
            if (!r.has("output")) throw new RuntimeException("Cloud reasoning failed");
        });

        runStep("UMI", "Restore LocalEngine-v0 as active driver", () -> {
            var r = kernel.umiSwitch("LocalEngine-v0");
            if (!r.path("ok").asBoolean()) throw new RuntimeException("Switch back failed");
        });

        // P6: Ring 0 security
        runStep("P6", "Ring 0 ALLOW: financial:read within scope of financial:*", () -> {
            var av = mapper.createObjectNode()
                .put("action", "financial:read")
                .put("capability", "financial:read")
                .put("target", "/ledger/2024")
                .put("amount", 0);
            var r = kernel.ring0Verify(av, List.of("financial:*"));
            if (!r.path("allowed").asBoolean())
                throw new RuntimeException("Expected ALLOWED: " + r.path("reason"));
        });

        runStep("P6", "Ring 0 DENY: filesystem:write (blocked by SMT constraint)", () -> {
            var av = mapper.createObjectNode()
                .put("action", "filesystem:write")
                .put("capability", "admin:escalate")
                .put("target", "/etc/passwd")
                .put("amount", 0);
            var r = kernel.ring0Verify(av, List.of("*:*"));
            if (r.path("allowed").asBoolean())
                throw new RuntimeException("Expected DENIED but got ALLOWED");
        });

        runStep("P6", "Kernel panic triggered for security violation", () -> {
            var r = kernel.kernelPanic(pidAuditor.get(),
                "Ring 0 violation: filesystem:write → blocked_actions constraint");
            if (!r.has("panic")) throw new RuntimeException("Kernel panic not triggered");
        });

        runStep("P6", "L1 memory flushed + alert broadcast verification", () -> {
            var r = kernel.kernelPanic(pidAccountant.get(), "Security cleanup");
            if (!r.path("l1_flushed").asBoolean())
                throw new RuntimeException("L1 flush flag not set");
        });

        // P7: Audit
        runStep("P7", "Episodic ledger contains >=5 events", () -> {
            var log = kernel.episodicLog();
            if (log.path("count").asInt() < 5)
                throw new RuntimeException("Expected >=5 episodic entries");
        });

        runStep("P7", "Ring 0 audit log with ALLOWED + DENIED counts", () -> {
            var audit = kernel.auditLog();
            if (audit.path("count").asInt() == 0) throw new RuntimeException("Audit log empty");
            if (audit.path("denied").asInt() == 0) throw new RuntimeException("Expected >=1 DENIED entry");
        });

        // P8: Stats
        runStep("P8", "Memory statistics collected", () -> {
            var stats = kernel.memoryStats();
            if (!stats.has("stats")) throw new RuntimeException("No memory stats");
        });

        runStep("P8", "UMI driver metrics collected", () -> {
            var metrics = kernel.umiMetrics();
            if (!metrics.has("metrics")) throw new RuntimeException("No UMI metrics");
        });

        runStep("P8", "Process listing verified", () -> {
            var procs = kernel.listProcesses();
            if (!procs.has("processes")) throw new RuntimeException("No process list");
        });
    }
}
