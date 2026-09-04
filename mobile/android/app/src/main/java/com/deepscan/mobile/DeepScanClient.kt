package com.deepscan.mobile

import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject
import java.io.IOException
import java.util.concurrent.TimeUnit

data class SearchResult(val path: String, val category: String, val snippet: String?, val score: Float)

/**
 * Thin client to the desktop DeepScan engine's `/api/*` endpoints (see
 * rust-engine/src/http.rs). Talks to a LAN address the user enters once —
 * e.g. http://192.168.1.42:51424 — never anything on the public internet.
 * This app does no indexing of its own; see mobile/README.md.
 */
class DeepScanClient(private val baseUrl: String) {

    private val http = OkHttpClient.Builder()
        .connectTimeout(4, TimeUnit.SECONDS)
        .readTimeout(15, TimeUnit.SECONDS)
        .build()

    fun search(textQuery: String): Result<List<SearchResult>> = runCatching {
        val body = JSONObject().put("text_query", textQuery).toString()
            .toRequestBody("application/json".toMediaType())
        val request = Request.Builder().url("$baseUrl/api/search").post(body).build()

        http.newCall(request).execute().use { resp ->
            if (!resp.isSuccessful) throw IOException("engine returned ${resp.code}")
            val json = JSONObject(resp.body?.string().orEmpty())
            parseResults(json.optJSONArray("results") ?: JSONArray())
        }
    }

    fun revealOnDesktop(path: String): Result<Unit> = runCatching {
        val body = JSONObject().put("path", path).toString()
            .toRequestBody("application/json".toMediaType())
        val request = Request.Builder().url("$baseUrl/api/reveal").post(body).build()
        http.newCall(request).execute().use { resp ->
            if (!resp.isSuccessful) throw IOException("reveal failed: ${resp.code}")
        }
    }

    private fun parseResults(arr: JSONArray): List<SearchResult> =
        (0 until arr.length()).map { i ->
            val obj = arr.getJSONObject(i)
            SearchResult(
                path = obj.getString("path"),
                category = obj.optString("category", ""),
                snippet = obj.optString("snippet", null),
                score = obj.optDouble("score", 0.0).toFloat(),
            )
        }
}
