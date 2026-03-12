mod activate;
mod config;
mod nic;
mod state;
mod tcp;
mod udp_dns;

pub unsafe fn init_userspace_network_activation(
    dma: morpheus_network::dma::DmaRegion,
    tsc_freq: u64,
) {
    activate::init_userspace_network_activation(dma, tsc_freq);
}
