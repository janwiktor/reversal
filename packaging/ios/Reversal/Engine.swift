// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The engine, linked into the app, and the way the page reaches it: the web view loads the page
// from the app's own scheme, and each of its requests goes to the engine as a call. There's no
// server and no port, so nothing outside the app can reach the engine, and iOS has no socket to
// close while the app is away.

import Foundation
import WebKit

/// The engine answers one request at a time, as its server does, and decoding a scan takes
/// seconds. So requests wait their turn on a queue of their own, off the main thread.
final class Engine {
    /// Where the page lives. The engine expects this origin, see `APP_ORIGIN` in `ui.rs`.
    static let page = URL(string: "reversal://app/")!
    static var scheme: String { page.scheme! }

    struct Response {
        let status: Int
        let headers: [String: String]
        let body: Data

        var json: [String: Any]? {
            (try? JSONSerialization.jsonObject(with: body)) as? [String: Any]
        }
    }

    static let json = ["Content-Type": "application/json"]

    private let queue = DispatchQueue(label: "engine", qos: .userInitiated)

    /// Opens `folder` in place of the one that was open, after the requests already waiting.
    /// `done` runs on the main queue.
    func open(_ folder: URL, done: @escaping (Bool) -> Void) {
        queue.async {
            let opened = folder.path.withCString { reversal_engine_open($0) }
            DispatchQueue.main.async { done(opened) }
        }
    }

    /// `done` runs on the main queue.
    func request(
        _ method: String, _ url: String, headers: [String: String] = [:], body: Data = Data(),
        done: @escaping (Response) -> Void
    ) {
        queue.async {
            let response = Self.ask(method, url, headers, body)
            DispatchQueue.main.async { done(response) }
        }
    }

    private static func ask(_ method: String, _ url: String, _ headers: [String: String], _ body: Data) -> Response {
        // A header's value can't hold a line break, and the engine reads one header per line.
        let lines = headers.map { "\($0.key): \($0.value.replacingOccurrences(of: "\n", with: " "))\n" }.joined()
        let answer = body.withUnsafeBytes { bytes in
            reversal_engine_request(
                method, url, lines, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count)!
        }
        defer { reversal_response_free(answer) }
        var fields: [String: String] = [:]
        for line in String(cString: answer.pointee.headers).split(separator: "\n") {
            guard let colon = line.firstIndex(of: ":") else { continue }
            fields[String(line[..<colon])] = line[line.index(after: colon)...].trimmingCharacters(in: .whitespaces)
        }
        return Response(
            status: Int(answer.pointee.status), headers: fields,
            body: Data(bytes: answer.pointee.body, count: answer.pointee.body_len))
    }
}

/// Passes the web view's requests for the app's scheme to the engine.
final class EngineSchemeHandler: NSObject, WKURLSchemeHandler {
    private let engine: Engine
    /// Requests the web view still waits for. Answering one it has stopped raises an exception.
    private var waiting = Set<ObjectIdentifier>()
    /// Set while another folder is being opened. The page on screen still belongs to the last
    /// one, and what it asked now would be answered by the new folder, so it gets no answer.
    var paused = false

    init(engine: Engine) {
        self.engine = engine
    }

    func webView(_: WKWebView, start task: WKURLSchemeTask) {
        let request = task.request
        guard let url = request.url,
            let parts = URLComponents(url: url, resolvingAgainstBaseURL: false)
        else { return task.didFailWithError(URLError(.badURL)) }
        guard !paused else { return task.didFailWithError(URLError(.cancelled)) }
        // As in a request line, still percent-encoded: the engine decodes it.
        let target = parts.percentEncodedPath + (parts.percentEncodedQuery.map { "?\($0)" } ?? "")
        waiting.insert(ObjectIdentifier(task))
        engine.request(
            request.httpMethod ?? "GET", target, headers: request.allHTTPHeaderFields ?? [:],
            body: Self.body(of: request)
        ) { [weak self] response in
            guard let self, self.waiting.remove(ObjectIdentifier(task)) != nil else { return }
            guard
                let head = HTTPURLResponse(
                    url: url, statusCode: response.status, httpVersion: "HTTP/1.1",
                    headerFields: response.headers)
            else { return task.didFailWithError(URLError(.badServerResponse)) }
            task.didReceive(head)
            task.didReceive(response.body)
            task.didFinish()
        }
    }

    func webView(_: WKWebView, stop task: WKURLSchemeTask) {
        waiting.remove(ObjectIdentifier(task))
    }

    /// The body comes as data or as a stream, depending on how the page sent it.
    private static func body(of request: URLRequest) -> Data {
        if let body = request.httpBody { return body }
        guard let stream = request.httpBodyStream else { return Data() }
        stream.open()
        defer { stream.close() }
        var body = Data()
        var buffer = [UInt8](repeating: 0, count: 64 * 1024)
        while stream.hasBytesAvailable {
            let read = stream.read(&buffer, maxLength: buffer.count)
            guard read > 0 else { break }
            body.append(buffer, count: read)
        }
        return body
    }
}
