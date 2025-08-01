<!-- Copyright 2023 OpenObserve Inc.

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program.  If not, see <http://www.gnu.org/licenses/>.
-->

<template>
  <q-page class="q-pa-none" style="min-height: inherit">
    <div v-if="!showDestinationEditor" style="height: calc(100vh - 112px); overflow-y: auto;">
      <div class="tw-flex tw-justify-between tw-items-center tw-px-4 tw-py-3"
      :class="store.state.theme == 'dark' ? 'o2-table-header-dark' : 'o2-table-header-light'"
      >
        <div class="q-table__title" data-test="alert-destinations-list-title">
            {{ t("pipeline_destinations.header") }}
          </div>
          <div class="tw-flex tw-justify-end">
            <q-input
            data-test="destination-list-search-input"
            v-model="filterQuery"
            borderless
            filled
            dense
            class="q-ml-auto no-border"
            :placeholder="t('alert_destinations.search')"
          >
            <template #prepend>
              <q-icon name="search" class="cursor-pointer" />
            </template>
          </q-input>
          <q-btn
            data-test="alert-destination-list-add-alert-btn"
            class="q-ml-md text-bold no-border"
            padding="sm lg"
            color="secondary"
            no-caps
            :label="t(`alert_destinations.add`)"
            @click="editDestination(null)"
          />
          </div>
      </div>
      <q-table
        data-test="alert-destinations-list-table"
        ref="qTable"
        :rows="destinations"
        :columns="columns"
        row-key="id"
        style="width: 100%"
        :pagination="pagination"
        :filter="filterQuery"
        :filter-method="filterData"
        class="o2-quasar-table"
        :class="store.state.theme == 'dark' ? 'o2-quasar-table-dark' : 'o2-quasar-table-light'"
      >
        <template #no-data>
          <template>
            <NoData />
          </template>
        </template>
        <template v-slot:body-cell-actions="props">
          <q-td :props="props">
            <q-btn
              :data-test="`alert-destination-list-${props.row.name}-update-destination`"
              icon="edit"
              class="q-ml-xs"
              padding="sm"
              unelevated
              size="sm"
              round
              flat
              :title="t('alert_destinations.edit')"
              @click="editDestination(props.row)"
            ></q-btn>
            <q-btn
              :data-test="`alert-destination-list-${props.row.name}-delete-destination`"
              :icon="outlinedDelete"
              class="q-ml-xs"
              padding="sm"
              unelevated
              size="sm"
              round
              flat
              :title="t('alert_destinations.delete')"
              @click="conformDeleteDestination(props.row)"
            ></q-btn>
          </q-td>
        </template>
        <template #top="scope">
          <QTablePagination
            :scope="scope"
            :pageTitle="t('pipeline_destinations.header')"
            :position="'top'"
            :resultTotal="resultTotal"
            :perPageOptions="perPageOptions"
            @update:changeRecordPerPage="changePagination"
          />
        </template>

        <template #bottom="scope">
          <QTablePagination
            :scope="scope"
            :position="'bottom'"
            :resultTotal="resultTotal"
            :perPageOptions="perPageOptions"
            @update:changeRecordPerPage="changePagination"
          />
        </template>
        <template v-slot:header="props">
            <q-tr :props="props">
              <!-- render the table headers -->
              <q-th
                v-for="col in props.cols"
                :key="col.name"
                :props="props"
                :class="col.classes"
                :style="col.style"
              >
                {{ col.label }}
              </q-th>
            </q-tr>
          </template>
      </q-table>
    </div>
    <div v-else>
      <AddDestination
        :is-alerts="false"
        :destination="editingDestination"
        :templates="templates"
        @cancel:hideform="toggleDestinationEditor"
        @get:destinations="getDestinations"
      />
    </div>

    <ConfirmDialog
      title="Delete Destination"
      message="Are you sure you want to delete destination?"
      @update:ok="deleteDestination"
      @update:cancel="cancelDeleteDestination"
      v-model="confirmDelete.visible"
    />
  </q-page>
</template>
<script lang="ts">
import { ref, onBeforeMount, onActivated, watch, defineComponent, onMounted } from "vue"; 
import type { Ref } from "vue";
import { useI18n } from "vue-i18n";
import { useQuasar, type QTableProps } from "quasar";
import NoData from "../shared/grid/NoData.vue";
import { getImageURL } from "@/utils/zincutils";
import AddDestination from "./AddDestination.vue";
import destinationService from "@/services/alert_destination";
import templateService from "@/services/alert_templates";
import { useStore } from "vuex";
import ConfirmDialog from "../ConfirmDialog.vue";
import { useRouter } from "vue-router";
import QTablePagination from "@/components/shared/grid/Pagination.vue";
import type { DestinationPayload } from "@/ts/interfaces";
import type { Template } from "@/ts/interfaces/index";

import { outlinedDelete } from "@quasar/extras/material-icons-outlined";

interface ConformDelete {
  visible: boolean;
  data: any;
}
export default defineComponent({
  name: "PageAlerts",
  components: { AddDestination, NoData, ConfirmDialog, QTablePagination },
  setup() {
    const qTable = ref();
    const store = useStore();
    const editingDestination: Ref<DestinationPayload | null> = ref(null);
    const { t } = useI18n();
    const q = useQuasar();
    const columns: any = ref<QTableProps["columns"]>([
      {
        name: "#",
        label: "#",
        field: "#",
        align: "left",
        style: "width: 67px",
      },
      {
        name: "name",
        field: "name",
        label: t("alert_destinations.name"),
        align: "left",
        sortable: true,
      },
      {
        name: "url",
        field: "url",
        label: t("alert_destinations.url"),
        align: "left",
        sortable: false,
      },
      {
        name: "method",
        field: "method",
        label: t("alert_destinations.method"),
        align: "left",
        sortable: true,
        style: "width: 150px",
      },
      {
        name: "actions",
        field: "actions",
        label: t("alert_destinations.actions"),
        align: "center",
        sortable: false,
        classes:'actions-column'
      },
    ]);
    const destinations: Ref<DestinationPayload[]> = ref([]);
    const templates: Ref<Template[]> = ref([
      { name: "test", body: "", type: "http" },
    ]);
    const confirmDelete: Ref<ConformDelete> = ref({
      visible: false,
      data: null,
    });
    const showDestinationEditor = ref(false);
    const router = useRouter();
    const filterQuery = ref("");
    const perPageOptions: any = [
      { label: "5", value: 5 },
      { label: "10", value: 10 },
      { label: "20", value: 20 },
      { label: "50", value: 50 },
      { label: "100", value: 100 },
      { label: "All", value: 0 },
    ];
    const resultTotal = ref(0);
    const selectedPerPage = ref(20);
    const pagination: any = ref({
      rowsPerPage: 20,
    });
    onActivated(() => {
      getTemplates();
      if (!destinations.value.length) getDestinations();
    });
    onBeforeMount(() => {
      getDestinations();
      getTemplates();
    });

    watch(
      () => router.currentRoute.value.query.action,
      (action) => {
        if (!action) showDestinationEditor.value = false;
      }
    );

    onMounted(()=>{
      updateRoute();
    })

    const getDestinations = () => {
      const dismiss = q.notify({
        spinner: true,
        message: "Please wait while loading destinations...",
      });
      destinationService
        .list({
          page_num: 1,
          page_size: 100000,
          sort_by: "name",
          desc: false,
          org_identifier: store.state.selectedOrganization.identifier,
          module: "pipeline",
        })
        .then((res) => {
          // res.data = res.data.filter(
          //   (destination: any) => destination.type == "external_destination",
          // );
          resultTotal.value = res.data.length;
          destinations.value = res.data.map((data: any, index: number) => ({
            ...data,
            "#": index + 1 <= 9 ? `0${index + 1}` : index + 1,
          }));
          updateRoute();
        })
        .catch((err) => {
          if(err.response.status != 403){
            q.notify({
              type: "negative",
              message: "Error while pulling destinations.",
              timeout: 2000,
            });
          }
          dismiss();
        })
        .finally(() => dismiss());
    };
    const getTemplates = () => {
      templateService
        .list({
          org_identifier: store.state.selectedOrganization.identifier,
        })
        .then((res) => (templates.value = res.data));
    };
    const updateRoute = () => {
      if (router.currentRoute.value.query.action === "add")
        editDestination(null);
      if (router.currentRoute.value.query.action === "update")
        editDestination(
          getDestinationByName(router.currentRoute.value.query.name as string)
        );
    };
    const getDestinationByName = (name: string) => {
      return destinations.value.find(
        (destination) => destination.name === name
      );
    };
    const editDestination = (destination: any) => {
      toggleDestinationEditor();
      resetEditingDestination();
      if (!destination) {
        router.push({
          name: "pipelineDestinations",
          query: {
            action: "add",
            org_identifier: store.state.selectedOrganization.identifier,
          },
        });
      } else {
        editingDestination.value = { ...destination };
        router.push({
          name: "pipelineDestinations",
          query: {
            action: "update",
            name: destination.name,
            org_identifier: store.state.selectedOrganization.identifier,
          },
        });
      }
    };
    const resetEditingDestination = () => {
      editingDestination.value = null;
    };
    const deleteDestination = () => {
      if (confirmDelete.value?.data?.name) {
        destinationService
          .delete({
            org_identifier: store.state.selectedOrganization.identifier,
            destination_name: confirmDelete.value.data.name,
          })
          .then(() => {
            q.notify({
              type: "positive",
              message: `Destination ${confirmDelete.value.data.name} deleted successfully`,
              timeout: 2000,
            });
            getDestinations();
          })
          .catch((err) => {
            if (err.response.data.code === 409) {
              const message =
                err.response.data?.message ||
                err.response.data?.error ||
                "Error while deleting destination";
              q.notify({
                type: "negative",
                message,
                timeout: 2000,
              });
            }
          });
      }
    };
    const conformDeleteDestination = (destination: any) => {
      confirmDelete.value.visible = true;
      confirmDelete.value.data = destination;
    };
    const cancelDeleteDestination = () => {
      confirmDelete.value.visible = false;
      confirmDelete.value.data = null;
    };
    const toggleDestinationEditor = () => {
      showDestinationEditor.value = !showDestinationEditor.value;
      if (!showDestinationEditor.value)
        router.push({
          name: "pipelineDestinations",
          query: {
            org_identifier: store.state.selectedOrganization.identifier,
          },
        });
    };
    const changePagination = (val: { label: string; value: any }) => {
      selectedPerPage.value = val.value;
      pagination.value.rowsPerPage = val.value;
      qTable.value.setPagination(pagination.value);
    };
    const filterData = (rows: any, terms: any) => {
      var filtered = [];
      terms = terms.toLowerCase();
      for (var i = 0; i < rows.length; i++) {
        if (rows[i]["name"].toLowerCase().includes(terms)) {
          filtered.push(rows[i]);
        }
      }
      return filtered;
    };

    const routeTo = (name: string) => {
      router.push({
        name: name,
        query: {
          action: "add",
          org_identifier: store.state.selectedOrganization.identifier,
        },
      });
    };

    return {
      t,
      qTable,
      showDestinationEditor,
      destinations,
      columns,
      editDestination,
      getImageURL,
      conformDeleteDestination,
      filterQuery,
      filterData,
      editingDestination,
      templates,
      toggleDestinationEditor,
      getDestinations,
      deleteDestination,
      cancelDeleteDestination,
      confirmDelete,
      changePagination,
      perPageOptions,
      resultTotal,
      pagination,
      outlinedDelete,
      routeTo,
      store,
    };
  },
});
</script>
<style lang=""></style>
