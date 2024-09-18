import Foundation
import Network

class UDPSender {
    var connection: NWConnection?
    let queue = DispatchQueue(label: "UDP Queue")
    let bufferSize: Int
    let duration: TimeInterval
    let bandwidthMbps: Double
    var timer: Timer?
    
    init(bufferSize: Int, duration: TimeInterval, bandwidthMbps: Double) {
        self.bufferSize = bufferSize
        self.duration = duration
        self.bandwidthMbps = bandwidthMbps
    }
    
    func startUDPSocket(host: String, port: UInt16) {
        let hostEndpoint = NWEndpoint.Host(host)
        let portEndpoint = NWEndpoint.Port(rawValue: port)!
        let connectionParams = NWParameters.udp
        
        connection = NWConnection(host: hostEndpoint, port: portEndpoint, using: connectionParams)
        connection?.start(queue: queue)
    }
    
    func sendUDPData() {
        guard let connection = connection else { return }
        
        // Calculate bytes per second based on bandwidth
        let bytesPerSecond = Int((bandwidthMbps * 1024 * 1024) / 8)
        // Calculate number of sends per second needed to achieve the desired bandwidth
        let sendsPerSecond = Double(bytesPerSecond) / Double(bufferSize)
        // Calculate interval between sends
        let interval = 1.0 / sendsPerSecond
        
        // Prepare data with the configured buffer size
        let data = Data(repeating: 0x00, count: bufferSize)
        
        // Schedule a loop with the calculated interval
        timer = Timer.scheduledTimer(withTimeInterval: interval, repeats: true) { [weak self] timer in
            // guard let self = self else { return }
            connection.send(content: data, completion: .contentProcessed { error in
                if let error = error {
                    print("Failed to send data: \(error)")
                    timer.invalidate() // Stop the loop if there is an error
                } else {
                    // print("Successfully sent \(self.bufferSize) bytes of data")
                }
            })
        }
        
        // Stop sending after the specified duration
        DispatchQueue.main.asyncAfter(deadline: .now() + duration) { [weak self] in
            self?.stopUDPSocket()
        }
    }
    
    func stopUDPSocket() {
        guard let connection = connection else { return }

        let data = Data(repeating: 0x01, count: bufferSize)
        connection.send(content: data, completion: .contentProcessed { error in
            if let error = error {
                print("Failed to send data: \(error)")
            } else {
                // print("Successfully sent \(self.bufferSize) bytes of data")
            }
        })

        connection?.cancel()
        timer?.invalidate()
        print("Stopped sending data")
    }
}

// Usage example
let udpSender = UDPSender(bufferSize: 2048, duration: 10.0, bandwidthMbps: 10.0) // 2048 bytes buffer, 10 seconds duration, 10 Mbps bandwidth
udpSender.startUDPSocket(host: "192.168.1.100", port: 8080) // Replace with actual server IP and port
udpSender.sendUDPData()

