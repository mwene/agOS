package com.agosdemo.config;

import com.agosdemo.service.AgosKernelClient;
import org.springframework.context.annotation.Bean;
import org.springframework.context.annotation.Configuration;

@Configuration
public class AppConfig {

    @Bean
    public AgosKernelClient kernelClient() {
        return new AgosKernelClient("http://127.0.0.1:7400");
    }
}
