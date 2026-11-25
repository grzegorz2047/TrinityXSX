/* vkcaps: print what vkd3d-proton's tiled-resources tier and feature level depend on, for every
 * Vulkan device (on the PS5 under prospero-win: the console's RADV through winevulkan).
 * Mirrors d3d12_device_determine_tiled_resources_tier() of vkd3d-proton 3.1 (libs/vkd3d/device.c).
 * Build (MSYS2 UCRT64): gcc -O2 -I<Vulkan-Headers>/include vkcaps.c -o vkcaps.exe
 * SPDX-License-Identifier: GPL-2.0-only */
#define VK_NO_PROTOTYPES
#include <vulkan/vulkan.h>
#include <windows.h>
#include <stdio.h>
#include <string.h>

#define P(name, value) printf("vkcaps:   %-44s %u\n", name, (unsigned)(value))

int main(void)
{
    HMODULE vk = LoadLibraryA("vulkan-1.dll");
    if (!vk) { printf("vkcaps: no vulkan-1.dll (%lu)\n", GetLastError()); return 1; }
    PFN_vkGetInstanceProcAddr gipa = (PFN_vkGetInstanceProcAddr)(void *)GetProcAddress(vk, "vkGetInstanceProcAddr");
    PFN_vkCreateInstance create_instance = (PFN_vkCreateInstance)gipa(NULL, "vkCreateInstance");

    VkApplicationInfo app = { VK_STRUCTURE_TYPE_APPLICATION_INFO, NULL, "vkcaps", 1, "vkcaps", 1, VK_API_VERSION_1_3 };
    VkInstanceCreateInfo ici = { VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO, NULL, 0, &app, 0, NULL, 0, NULL };
    VkInstance instance;
    VkResult vr = create_instance(&ici, NULL, &instance);
    if (vr != VK_SUCCESS) { printf("vkcaps: vkCreateInstance %d\n", vr); return 1; }

#define I(fn) PFN_##fn fn = (PFN_##fn)gipa(instance, #fn)
    I(vkEnumeratePhysicalDevices);
    I(vkGetPhysicalDeviceProperties2);
    I(vkGetPhysicalDeviceFeatures2);
    I(vkGetPhysicalDeviceQueueFamilyProperties);
    I(vkEnumerateDeviceExtensionProperties);

    uint32_t n = 8;
    VkPhysicalDevice devs[8];
    vkEnumeratePhysicalDevices(instance, &n, devs);
    printf("vkcaps: %u device(s)\n", n);
    for (uint32_t d = 0; d < n; d++)
    {
        VkPhysicalDeviceVulkan12Properties p12 = { VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_2_PROPERTIES };
        VkPhysicalDeviceVulkan13Properties p13 = { VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_PROPERTIES, &p12 };
        VkPhysicalDeviceProperties2 props = { VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2, &p13 };
        vkGetPhysicalDeviceProperties2(devs[d], &props);
        VkPhysicalDeviceFeatures2 feats = { VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2 };
        vkGetPhysicalDeviceFeatures2(devs[d], &feats);
        const VkPhysicalDeviceFeatures *f = &feats.features;
        const VkPhysicalDeviceSparseProperties *s = &props.properties.sparseProperties;
        uint32_t api = props.properties.apiVersion;

        printf("vkcaps: device %u: %s, Vulkan %u.%u.%u, driver %#x, vendor %#x device %#x\n", d,
                props.properties.deviceName, VK_API_VERSION_MAJOR(api), VK_API_VERSION_MINOR(api),
                VK_API_VERSION_PATCH(api), props.properties.driverVersion, props.properties.vendorID,
                props.properties.deviceID);
        printf("vkcaps:  tiled resources tier 1 (sparse binding):\n");
        P("sparseBinding", f->sparseBinding);
        P("sparseResidencyAliased", f->sparseResidencyAliased);
        P("sparseResidencyBuffer", f->sparseResidencyBuffer);
        P("sparseResidencyImage2D", f->sparseResidencyImage2D);
        P("residencyStandard2DBlockShape", s->residencyStandard2DBlockShape);
        printf("vkcaps:  tier 2 (needed for feature level 12_0):\n");
        P("shaderResourceResidency", f->shaderResourceResidency);
        P("shaderResourceMinLod", f->shaderResourceMinLod);
        P("residencyAlignedMipSize (must be 0)", s->residencyAlignedMipSize);
        P("residencyNonResidentStrict", s->residencyNonResidentStrict);
        P("filterMinmaxSingleComponentFormats", p12.filterMinmaxSingleComponentFormats);
        printf("vkcaps:  tier 3 / 4:\n");
        P("sparseResidencyImage3D", f->sparseResidencyImage3D);
        P("residencyStandard3DBlockShape", s->residencyStandard3DBlockShape);
        printf("vkcaps:  others (feature level 11_1, buffers):\n");
        P("logicOp", f->logicOp);
        P("vertexPipelineStoresAndAtomics", f->vertexPipelineStoresAndAtomics);
        P("maxPerStageDescriptorStorageBuffers", props.properties.limits.maxPerStageDescriptorStorageBuffers);
        P("maxPerStageDescriptorStorageImages", props.properties.limits.maxPerStageDescriptorStorageImages);
        printf("vkcaps:   %-44s %llu\n", "maxBufferSize", (unsigned long long)p13.maxBufferSize);

        uint32_t qn = 16;
        VkQueueFamilyProperties q[16];
        vkGetPhysicalDeviceQueueFamilyProperties(devs[d], &qn, q);
        for (uint32_t i = 0; i < qn; i++)
            printf("vkcaps:  queue family %u: flags %#x (sparse binding %u), %u queue(s)\n", i, q[i].queueFlags,
                    !!(q[i].queueFlags & VK_QUEUE_SPARSE_BINDING_BIT), q[i].queueCount);

        static VkExtensionProperties ext[512];
        uint32_t en = 512;
        vkEnumerateDeviceExtensionProperties(devs[d], NULL, &en, ext);
        const char *want[] = { "VK_KHR_fragment_shader_barycentric", "VK_EXT_mesh_shader", "VK_EXT_shader_object",
                               "VK_EXT_descriptor_buffer", "VK_KHR_shader_float_controls2", "VK_EXT_fragment_shader_interlock" };
        for (unsigned w = 0; w < sizeof(want) / sizeof(want[0]); w++)
        {
            unsigned have = 0;
            for (uint32_t e = 0; e < en; e++)
                have |= !strcmp(ext[e].extensionName, want[w]);
            P(want[w], have);
        }
    }
    fflush(stdout);
    return 0;
}
