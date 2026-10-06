// Minimal Unix-domain-socket client for the fand daemon: one JSON line out,
// one JSON line back. Blocking I/O runs off the main actor.

import Foundation

public enum DaemonError: Error, LocalizedError, Equatable {
    /// Socket missing or nobody listening: the daemon isn't installed/running.
    case notRunning
    case io(String)
    case daemon(String)
    case badResponse(String)

    public var errorDescription: String? {
        switch self {
        case .notRunning: "The fan daemon isn't running."
        case let .io(m): "Connection error: \(m)"
        case let .daemon(m): m
        case let .badResponse(m): "Unexpected response from daemon: \(m)"
        }
    }
}

public struct DaemonClient: Sendable {
    /// `MACFANOPTIMIZER_SOCKET` overrides the path (useful with `fand --dry-run --socket ...`).
    public static let defaultSocketPath =
        ProcessInfo.processInfo.environment["MACFANOPTIMIZER_SOCKET"] ?? "/var/run/macfanoptimizer.sock"

    public let socketPath: String
    public let timeout: TimeInterval

    public init(socketPath: String = DaemonClient.defaultSocketPath, timeout: TimeInterval = 3) {
        self.socketPath = socketPath
        self.timeout = timeout
    }

    public func send(_ request: Request) async throws -> Response {
        let payload = try JSONEncoder().encode(request)
        let path = socketPath
        let timeout = timeout
        let line = try await Task.detached(priority: .utility) {
            try Self.roundTrip(path: path, payload: payload, timeout: timeout)
        }.value
        let response: Response
        do {
            response = try JSONDecoder.daemon.decode(Response.self, from: line)
        } catch {
            throw DaemonError.badResponse(String(describing: error))
        }
        if case let .error(message) = response { throw DaemonError.daemon(message) }
        return response
    }

    public func status() async throws -> Status {
        guard case let .status(s) = try await send(.status) else { throw DaemonError.badResponse("expected status") }
        return s
    }

    static func roundTrip(path: String, payload: Data, timeout: TimeInterval) throws -> Data {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw DaemonError.io(String(cString: strerror(errno))) }
        defer { close(fd) }

        var tv = timeval(tv_sec: Int(timeout), tv_usec: 0)
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        var noSigPipe: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &noSigPipe, socklen_t(MemoryLayout<Int32>.size))

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let pathBytes = Array(path.utf8)
        guard pathBytes.count < MemoryLayout.size(ofValue: addr.sun_path) else { throw DaemonError.io("socket path too long") }
        withUnsafeMutableBytes(of: &addr.sun_path) { buf in
            buf.copyBytes(from: pathBytes)
            buf[pathBytes.count] = 0
        }
        let connected = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        if connected != 0 {
            if errno == ENOENT || errno == ECONNREFUSED { throw DaemonError.notRunning }
            throw DaemonError.io(String(cString: strerror(errno)))
        }

        var out = payload
        out.append(0x0A)
        try out.withUnsafeBytes { raw in
            var sent = 0
            while sent < raw.count {
                let n = write(fd, raw.baseAddress! + sent, raw.count - sent)
                if n <= 0 { throw DaemonError.io(String(cString: strerror(errno))) }
                sent += n
            }
        }

        var response = Data()
        var buf = [UInt8](repeating: 0, count: 16 * 1024)
        while !response.contains(0x0A) {
            let n = read(fd, &buf, buf.count)
            if n < 0 { throw DaemonError.io(String(cString: strerror(errno))) }
            if n == 0 { break }
            response.append(contentsOf: buf[0..<n])
            if response.count > 4 * 1024 * 1024 { throw DaemonError.badResponse("response too large") }
        }
        if let nl = response.firstIndex(of: 0x0A) { response = response[..<nl] }
        guard !response.isEmpty else { throw DaemonError.badResponse("empty response") }
        return response
    }
}
