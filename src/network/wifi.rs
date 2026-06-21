use anyhow::{anyhow, Result};
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs};
use esp_idf_svc::wifi::{
    AccessPointConfiguration, AuthMethod, ClientConfiguration, Configuration,
    EspWifi,
};
use log::info;

pub fn get_credentials(
    nvs_partition: &EspDefaultNvsPartition,
) -> (String, String) {
    let Ok(nvs) = EspNvs::new(nvs_partition.clone(), "wifi_creds", true) else {
        return (
            crate::config::WIFI_SSID.to_string(),
            crate::config::WIFI_PASS.to_string(),
        );
    };

    let mut ssid_buf = [0u8; 64];
    let mut pass_buf = [0u8; 64];

    let ssid = if let Ok(Some(s)) = nvs.get_str("ssid", &mut ssid_buf) {
        s.to_string()
    } else {
        crate::config::WIFI_SSID.to_string()
    };

    let pass = if let Ok(Some(p)) = nvs.get_str("pass", &mut pass_buf) {
        p.to_string()
    } else {
        crate::config::WIFI_PASS.to_string()
    };

    (ssid, pass)
}

pub fn save_credentials(
    nvs_partition: EspDefaultNvsPartition,
    ssid: &str,
    pass: &str,
) -> Result<()> {
    let nvs = EspNvs::new(nvs_partition, "wifi_creds", true)
        .map_err(|e| anyhow!("Failed to open NVS: {:?}", e))?;
    nvs.set_str("ssid", ssid)
        .map_err(|e| anyhow!("Failed to save ssid: {:?}", e))?;
    nvs.set_str("pass", pass)
        .map_err(|e| anyhow!("Failed to save pass: {:?}", e))?;
    Ok(())
}

pub fn connect_wifi(
    wifi: &mut EspWifi<'static>,
    nvs_partition: &EspDefaultNvsPartition,
) -> Result<()> {
    let (ssid, pass) = get_credentials(nvs_partition);

    let client_config = ClientConfiguration {
        ssid: ssid
            .as_str()
            .try_into()
            .map_err(|_| anyhow!("Invalid SSID length"))?,
        password: pass
            .as_str()
            .try_into()
            .map_err(|_| anyhow!("Invalid Password length"))?,
        auth_method: if pass.is_empty() {
            AuthMethod::None
        } else {
            AuthMethod::WPA2Personal
        },
        ..Default::default()
    };

    let ap_config = AccessPointConfiguration {
        ssid: crate::config::AP_SSID
            .try_into()
            .map_err(|_| anyhow!("Invalid Matrix-Hub AP SSID length"))?,
        password: crate::config::AP_PASS
            .try_into()
            .map_err(|_| anyhow!("Invalid Matrix-Hub AP Pass length"))?,
        auth_method: {
            #[allow(
                clippy::const_is_empty,
                reason = "AP_PASS is a compile-time constant"
            )]
            if crate::config::AP_PASS.is_empty() {
                AuthMethod::None
            } else {
                AuthMethod::WPA2Personal
            }
        },
        ..Default::default()
    };

    let _ = wifi.disconnect();

    wifi.set_configuration(&Configuration::Mixed(client_config, ap_config))
        .map_err(|e| anyhow!("Could not set wifi config: {:?}", e))?;

    // Configure the AP's DHCP server to provide the ESP32 as the DNS server
    use esp_idf_svc::handle::RawHandle;
    use esp_idf_svc::sys::{
        esp_netif_dhcp_option_id_t_ESP_NETIF_DOMAIN_NAME_SERVER,
        esp_netif_dhcp_option_mode_t_ESP_NETIF_OP_SET, esp_netif_dhcps_option,
        esp_netif_dns_info_t, esp_netif_dns_type_t_ESP_NETIF_DNS_MAIN,
        esp_netif_get_ip_info, esp_netif_ip_info_t, esp_netif_set_dns_info,
    };

    #[allow(unsafe_code, reason = "Calling C FFI for DHCP options")]
    unsafe {
        let netif_handle = wifi.ap_netif().handle() as *mut _;

        // Get the AP's IP
        let mut ip_info: esp_netif_ip_info_t = std::mem::zeroed();
        esp_netif_get_ip_info(netif_handle, &mut ip_info);

        // Set the Main DNS server to be the AP's IP
        let mut dns_info: esp_netif_dns_info_t = std::mem::zeroed();
        dns_info.ip.type_ = 0; // ESP_IPADDR_TYPE_V4
        dns_info.ip.u_addr.ip4 = ip_info.ip;

        esp_netif_set_dns_info(
            netif_handle,
            esp_netif_dns_type_t_ESP_NETIF_DNS_MAIN,
            &mut dns_info,
        );

        // Tell the DHCP server to advertise the Main DNS server
        let mut dhcps_dns_value: u8 = 1; // 1 = OFFER_DNS_MAIN
        let _ = esp_netif_dhcps_option(
            netif_handle,
            esp_netif_dhcp_option_mode_t_ESP_NETIF_OP_SET,
            esp_netif_dhcp_option_id_t_ESP_NETIF_DOMAIN_NAME_SERVER,
            &mut dhcps_dns_value as *mut _ as *mut _,
            1,
        );
    }

    info!("Starting wifi...");
    wifi.start()
        .map_err(|e| anyhow!("Could not start wifi: {:?}", e))?;

    info!("Connecting to WiFi SSID: {}", ssid);
    wifi.connect()
        .map_err(|e| anyhow!("Could not connect wifi: {:?}", e))?;

    info!("WiFi connection initiated asynchronously!");
    Ok(())
}
