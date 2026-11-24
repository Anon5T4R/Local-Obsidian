package com.openobsidian.android.data.update

import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test

class UpdateNetTest {

    private val newUrl = UpdateNet.latestUrl(ReleasePicker.REPO_PRIMARY)
    private val oldUrl = UpdateNet.latestUrl(ReleasePicker.REPO_FALLBACK)

    @Test
    fun repo_novo_que_responde_nao_consulta_o_antigo() {
        val asked = mutableListOf<String>()
        val r = UpdateNet.fetchLatest { url -> asked += url; "{}" }
        assertEquals(ReleasePicker.REPO_PRIMARY, r.repo)
        assertEquals(listOf(newUrl), asked)
    }

    @Test
    fun _404_no_novo_cai_no_antigo() {
        val asked = mutableListOf<String>()
        val r = UpdateNet.fetchLatest { url ->
            asked += url
            if (url == newUrl) throw UpdateException(UpdateFailure.HTTP, 404) else "{\"tag_name\":\"v1.0.0\"}"
        }
        assertEquals(ReleasePicker.REPO_FALLBACK, r.repo)
        assertEquals(listOf(newUrl, oldUrl), asked)
    }

    @Test
    fun outros_erros_no_novo_nao_caem_no_antigo() {
        listOf(
            UpdateException(UpdateFailure.HTTP, 403),
            UpdateException(UpdateFailure.HTTP, 429),
            UpdateException(UpdateFailure.HTTP, 500),
            UpdateException(UpdateFailure.HTTP, 503),
            UpdateException(UpdateFailure.NETWORK),
        ).forEach { error ->
            val asked = mutableListOf<String>()
            try {
                UpdateNet.fetchLatest { url -> asked += url; throw error }
                fail("esperava UpdateException")
            } catch (e: UpdateException) {
                assertEquals(error.failure, e.failure)
                assertEquals(error.code, e.code)
            }
            assertEquals(listOf(newUrl), asked)
        }
    }

    @Test
    fun erro_do_antigo_depois_do_404_sobe() {
        try {
            UpdateNet.fetchLatest { throw UpdateException(UpdateFailure.HTTP, 404) }
            fail("esperava UpdateException")
        } catch (e: UpdateException) {
            assertEquals(404, e.code)
        }
    }
}
