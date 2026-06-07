package com.agosdemo.controller;

import com.agosdemo.service.DemoOrchestrator;
import org.springframework.stereotype.Controller;
import org.springframework.ui.Model;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.PostMapping;
import org.springframework.web.bind.annotation.ResponseBody;

import java.util.Map;

@Controller
public class WebController {

    private final DemoOrchestrator orchestrator;

    public WebController(DemoOrchestrator orchestrator) {
        this.orchestrator = orchestrator;
    }

    @GetMapping("/")
    public String index(Model model) {
        model.addAttribute("kernelAlive", orchestrator.isKernelAlive());
        model.addAttribute("steps", orchestrator.getSteps());
        return "demo";
    }

    @PostMapping("/api/demo/run")
    @ResponseBody
    public Map<String, Object> runDemo() {
        orchestrator.runFullDemo();
        long passed = orchestrator.getSteps().stream().filter(DemoOrchestrator.DemoStep::success).count();
        long failed = orchestrator.getSteps().size() - passed;
        return Map.of(
            "total", orchestrator.getSteps().size(),
            "passed", passed,
            "failed", failed,
            "running", true
        );
    }

    @GetMapping("/api/demo/status")
    @ResponseBody
    public Map<String, Object> status() {
        long passed = orchestrator.getSteps().stream().filter(DemoOrchestrator.DemoStep::success).count();
        long failed = orchestrator.getSteps().size() - passed;
        return Map.of(
            "total", orchestrator.getSteps().size(),
            "passed", passed,
            "failed", failed,
            "kernelAlive", orchestrator.isKernelAlive(),
            "steps", orchestrator.getSteps()
        );
    }

    @PostMapping("/api/demo/reset")
    @ResponseBody
    public Map<String, Object> reset() {
        orchestrator.getSteps().clear();
        return Map.of("ok", true);
    }
}
