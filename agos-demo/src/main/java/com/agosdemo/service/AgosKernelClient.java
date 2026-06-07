package com.agosdemo.service;

import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.core.type.TypeReference;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import org.springframework.web.client.RestClient;

import java.util.*;

public class AgosKernelClient {

    private final String baseUrl;
    private final RestClient client;
    private final ObjectMapper mapper;

    public AgosKernelClient(String baseUrl) {
        this.baseUrl = baseUrl;
        this.client = RestClient.builder().baseUrl(baseUrl).build();
        this.mapper = new ObjectMapper();
    }

    public boolean health() {
        try {
            var resp = client.get().uri("/health").retrieve().body(JsonNode.class);
            return resp != null && "ok".equals(resp.path("status").asText());
        } catch (Exception e) {
            return false;
        }
    }

    // ── Kernel ─────────────────────────────────────────
    public JsonNode spawnProcess(ObjectNode manifest, String parentPid) {
        var body = mapper.createObjectNode();
        body.set("manifest", manifest);
        if (parentPid != null) body.put("parent_pid", parentPid);
        return client.post().uri("/kernel/spawn").body(body).retrieve().body(JsonNode.class);
    }

    public JsonNode killProcess(String pid) {
        var body = mapper.createObjectNode().put("pid", pid);
        return client.post().uri("/kernel/kill").body(body).retrieve().body(JsonNode.class);
    }

    public JsonNode listProcesses() {
        return client.get().uri("/kernel/processes").retrieve().body(JsonNode.class);
    }

    public JsonNode getProcess(String pid) {
        return client.get().uri("/kernel/process/{pid}", pid).retrieve().body(JsonNode.class);
    }

    public JsonNode kernelPanic(String pid, String reason) {
        var body = mapper.createObjectNode().put("pid", pid).put("reason", reason);
        return client.post().uri("/kernel/panic").body(body).retrieve().body(JsonNode.class);
    }

    // ── Memory ─────────────────────────────────────────
    public JsonNode writeL1(String pid, List<JsonNode> data) {
        var body = mapper.createObjectNode().put("pid", pid);
        var arr = body.putArray("data");
        data.forEach(arr::add);
        return client.post().uri("/memory/l1/write").body(body).retrieve().body(JsonNode.class);
    }

    public JsonNode readL1(String pid) {
        return client.get().uri("/memory/l1/{pid}", pid).retrieve().body(JsonNode.class);
    }

    public JsonNode compressL2(String pid) {
        var body = mapper.createObjectNode().put("pid", pid);
        return client.post().uri("/memory/l2/compress").body(body).retrieve().body(JsonNode.class);
    }

    public JsonNode storeVector(List<Double> vector, ObjectNode metadata) {
        var body = mapper.createObjectNode();
        var arr = body.putArray("vector");
        vector.forEach(arr::add);
        body.set("metadata", metadata);
        return client.post().uri("/memory/vector").body(body).retrieve().body(JsonNode.class);
    }

    public JsonNode pageIn(List<Double> query, int topK) {
        var body = mapper.createObjectNode();
        var arr = body.putArray("query_vector");
        query.forEach(arr::add);
        body.put("top_k", topK);
        return client.post().uri("/memory/page").body(body).retrieve().body(JsonNode.class);
    }

    public JsonNode memoryStats() {
        return client.get().uri("/memory/stats").retrieve().body(JsonNode.class);
    }

    public JsonNode episodicLog() {
        return client.get().uri("/memory/episodic").retrieve().body(JsonNode.class);
    }

    // ── cIPC ────────────────────────────────────────────
    public JsonNode routeMessage(String from, String to, ObjectNode payload) {
        var body = mapper.createObjectNode();
        body.put("from", from).put("to", to);
        body.set("payload", payload);
        return client.post().uri("/cipc/route").body(body).retrieve().body(JsonNode.class);
    }

    public JsonNode cipcHistory() {
        return client.get().uri("/cipc/history").retrieve().body(JsonNode.class);
    }

    // ── Security ────────────────────────────────────────
    public JsonNode ring0Verify(ObjectNode actionVector, List<String> permissions) {
        var body = mapper.createObjectNode();
        body.set("action_vector", actionVector);
        var arr = body.putArray("permissions");
        permissions.forEach(arr::add);
        return client.post().uri("/security/verify").body(body).retrieve().body(JsonNode.class);
    }

    public JsonNode auditLog() {
        return client.get().uri("/security/audit").retrieve().body(JsonNode.class);
    }

    // ── UMI ─────────────────────────────────────────────
    public JsonNode umiReason(List<String> promptVector) {
        var body = mapper.createObjectNode();
        var arr = body.putArray("prompt_vector");
        promptVector.forEach(arr::add);
        return client.post().uri("/umi/reason").body(body).retrieve().body(JsonNode.class);
    }

    public JsonNode umiEmbed(List<String> data) {
        var body = mapper.createObjectNode();
        var arr = body.putArray("data");
        data.forEach(arr::add);
        return client.post().uri("/umi/embed").body(body).retrieve().body(JsonNode.class);
    }

    public JsonNode umiCompress(List<String> data) {
        var body = mapper.createObjectNode();
        var arr = body.putArray("data");
        data.forEach(arr::add);
        return client.post().uri("/umi/compress").body(body).retrieve().body(JsonNode.class);
    }

    public JsonNode umiSwitch(String driverName) {
        var body = mapper.createObjectNode().put("driver_name", driverName);
        return client.post().uri("/umi/switch").body(body).retrieve().body(JsonNode.class);
    }

    public JsonNode umiMetrics() {
        return client.get().uri("/umi/metrics").retrieve().body(JsonNode.class);
    }

    // ── Sandbox ─────────────────────────────────────────
    public JsonNode installApp(String name) {
        var body = mapper.createObjectNode().put("name", name).put("aapps_path", "aapps");
        return client.post().uri("/sandbox/install").body(body).retrieve().body(JsonNode.class);
    }

    public JsonNode launchApp(String name, String parentPid) {
        var body = mapper.createObjectNode().put("name", name);
        if (parentPid != null) body.put("parent_pid", parentPid);
        return client.post().uri("/sandbox/launch").body(body).retrieve().body(JsonNode.class);
    }

    public JsonNode listApps() {
        return client.get().uri("/sandbox/apps").retrieve().body(JsonNode.class);
    }
}
