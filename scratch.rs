use esp_idf_sys::{esp_netif_dns_info_t, esp_ip_addr_t, esp_ip4_addr_t};

fn main() {
    let mut d: esp_netif_dns_info_t = unsafe { std::mem::zeroed() };
    d.ip.u_addr.ip4.addr = 0;
    d.ip.type_ = 0;
}
